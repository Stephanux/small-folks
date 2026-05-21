//! Worker userspace eBPF — charge le programme XDP et gère les BPF maps.
//!
//! ## Démarrage automatique
//! Si `EBPF_ENABLED=true` dans `.env`, ce worker est lancé depuis `main.rs`
//! comme tâche Tokio de fond, au même titre que `mqtt_worker`.
//!
//! ## Variables .env
//! ```
//! EBPF_ENABLED=true
//! EBPF_INTERFACE=eth0          ← interface réseau à surveiller
//! EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
//! EBPF_RATE_LIMIT=100          ← SYN max par fenêtre
//! EBPF_WINDOW_SECS=60          ← fenêtre de temps en secondes
//! EBPF_AUTO_UNBLOCK_SECS=300   ← déblocage automatique après N secondes (0 = jamais)
//! ```
//!
//! ## Prérequis système
//! - Linux kernel ≥ 5.8
//! - Privilège CAP_BPF (ou root)
//! - Programme eBPF compilé : voir ebpf-firewall/README.md

use aya::{
    maps::{Array, HashMap as BpfHashMap},
    programs::{Xdp, XdpFlags},
    Bpf, BpfLoader,
};
use sqlx::MySqlPool;
use std::net::Ipv4Addr;
use std::time::Duration;

// ── Point d'entrée ────────────────────────────────────────────────────────────

pub async fn start(pool: MySqlPool) {
    let interface   = std::env::var("EBPF_INTERFACE")
        .unwrap_or_else(|_| "eth0".to_string());
    let program_path = std::env::var("EBPF_PROGRAM")
        .unwrap_or_else(|_|
            "./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall"
            .to_string()
        );
    let rate_limit = std::env::var("EBPF_RATE_LIMIT")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<u64>().unwrap_or(100);
    let window_secs = std::env::var("EBPF_WINDOW_SECS")
        .unwrap_or_else(|_| "60".to_string())
        .parse::<u64>().unwrap_or(60);
    let auto_unblock_secs = std::env::var("EBPF_AUTO_UNBLOCK_SECS")
        .unwrap_or_else(|_| "300".to_string())
        .parse::<u64>().unwrap_or(300);

    println!("[ebpf_worker] Interface  : {}", interface);
    println!("[ebpf_worker] Programme  : {}", program_path);
    println!("[ebpf_worker] Rate limit : {} SYN / {}s", rate_limit, window_secs);

    match run(pool, &interface, &program_path, rate_limit, window_secs, auto_unblock_secs).await {
        Ok(_)  => println!("[ebpf_worker] Arrêté proprement"),
        Err(e) => eprintln!("[ebpf_worker] ✗ Erreur fatale : {}", e),
    }
}

// ── Boucle principale ─────────────────────────────────────────────────────────

async fn run(
    pool:              MySqlPool,
    interface:         &str,
    program_path:      &str,
    rate_limit:        u64,
    window_secs:       u64,
    auto_unblock_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {

    // ── 1. Lire le binaire eBPF compilé ───────────────────────────────────────
    let program_bytes = std::fs::read(program_path)
        .map_err(|e| format!(
            "Impossible de lire '{}' : {}. Avez-vous compilé ebpf-firewall ?", program_path, e
        ))?;

    // ── 2. Charger le programme dans le kernel ────────────────────────────────
    let mut bpf = BpfLoader::new()
        .load(&program_bytes)
        .map_err(|e| format!("Erreur chargement eBPF : {}", e))?;

    // ── 3. Configurer les maps avant l'attachement ────────────────────────────
    {
        let mut config: Array<_, u64> = Array::try_from(
            bpf.map_mut("CONFIG").expect("Map CONFIG introuvable")
        )?;
        config.set(0, rate_limit, 0)?;          // index 0 = rate_limit
        config.set(1, window_secs * 1_000_000_000, 0)?; // index 1 = window en ns
        println!("[ebpf_worker] ✓ CONFIG initialisée");
    }

    // ── 4. Attacher le programme XDP à l'interface ────────────────────────────
    let program: &mut Xdp = bpf
        .program_mut("xdp_firewall")
        .expect("Programme xdp_firewall introuvable")
        .try_into()?;

    program.load()?;
    // XdpFlags::default() = SKB mode (compatible toutes interfaces)
    // XdpFlags::DRV_MODE = driver mode (plus performant, pas partout)
    program.attach(interface, XdpFlags::default())
        .map_err(|e| format!(
            "Erreur attachement XDP sur {} : {}. Êtes-vous root / CAP_BPF ?", interface, e
        ))?;

    println!("[ebpf_worker] ✓ Programme XDP attaché sur {}", interface);
    println!("[ebpf_worker] ✓ Protection active — en surveillance...");

    // ── 5. Boucle de monitoring (toutes les 5s) ───────────────────────────────
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        // Lire les statistiques globales
        if let Ok(stats) = read_stats(&bpf) {
            println!(
                "[ebpf_worker] Stats → paquets:{} drops:{} syn:{}",
                stats[0], stats[1], stats[2]
            );
        }

        // Synchroniser la blacklist eBPF → MySQL
        if let Err(e) = sync_blacklist(&bpf, &pool, auto_unblock_secs).await {
            eprintln!("[ebpf_worker] Erreur sync blacklist : {}", e);
        }

        // Déblocage automatique des IPs expirées
        if auto_unblock_secs > 0 {
            if let Err(e) = auto_unblock(&mut bpf, &pool, auto_unblock_secs).await {
                eprintln!("[ebpf_worker] Erreur auto_unblock : {}", e);
            }
        }
    }
}

// ── Lecture des statistiques globales ─────────────────────────────────────────

fn read_stats(bpf: &Bpf) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
    let stats: Array<_, u64> = Array::try_from(
        bpf.map("STATS").ok_or("Map STATS introuvable")?
    )?;
    Ok(vec![
        stats.get(&0, 0).unwrap_or(0),  // paquets vus
        stats.get(&1, 0).unwrap_or(0),  // drops totaux
        stats.get(&2, 0).unwrap_or(0),  // SYN totaux
    ])
}

// ── Synchronisation blacklist eBPF → MySQL ────────────────────────────────────
//
// Les IPs bloquées par le kernel sont reflétées en base MySQL pour :
// - Affichage via plugin_sql (GET /security/blacklist)
// - Audit et historique
// - Déblocage manuel via interface web

async fn sync_blacklist(
    bpf:               &Bpf,
    pool:              &MySqlPool,
    auto_unblock_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {

    let blacklist: BpfHashMap<_, u32, u64> = BpfHashMap::try_from(
        bpf.map("BLACKLIST").ok_or("Map BLACKLIST introuvable")?
    )?;

    // Parcourir les IPs bloquées dans la map eBPF
    for item in blacklist.iter() {
        let (ip_net, _ts_ns) = item?;
        let ip_str  = Ipv4Addr::from(u32::from_be(ip_net)).to_string();
        let blocked_at = chrono::Local::now().naive_local();
        let unblock_at = if auto_unblock_secs > 0 {
            Some(blocked_at + chrono::Duration::seconds(auto_unblock_secs as i64))
        } else {
            None
        };

        // INSERT OR IGNORE — on ne réinsère pas si déjà en base
        sqlx::query(
            "INSERT IGNORE INTO ebpf_blacklist
             (ip_address, blocked_at, unblock_at, reason)
             VALUES (?, ?, ?, ?)"
        )
        .bind(&ip_str)
        .bind(blocked_at)
        .bind(unblock_at)
        .bind("rate_limit_exceeded")
        .execute(pool)
        .await?;
    }

    Ok(())
}

// ── Déblocage automatique des IPs expirées ────────────────────────────────────

async fn auto_unblock(
    bpf:               &mut Bpf,
    pool:              &MySqlPool,
    _auto_unblock_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {

    let now = chrono::Local::now().naive_local();

    // Récupérer les IPs à débloquer depuis MySQL
    let rows = sqlx::query(
        "SELECT ip_address FROM ebpf_blacklist
         WHERE unblocked_at IS NULL
           AND unblock_at IS NOT NULL
           AND unblock_at <= ?"
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() { return Ok(()); }

    let mut blacklist: BpfHashMap<_, u32, u64> = BpfHashMap::try_from(
        bpf.map_mut("BLACKLIST").ok_or("Map BLACKLIST introuvable")?
    )?;

    for row in &rows {
        let ip_str: String = sqlx::Row::get(row, "ip_address");
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            let ip_net = u32::from(ip).to_be();

            // Supprimer de la map eBPF kernel
            blacklist.remove(&ip_net).ok();

            // Marquer comme débloqué en MySQL
            sqlx::query(
                "UPDATE ebpf_blacklist
                 SET unblocked_at = ?
                 WHERE ip_address = ? AND unblocked_at IS NULL"
            )
            .bind(now)
            .bind(&ip_str)
            .execute(pool)
            .await?;

            println!("[ebpf_worker] ✓ IP débloquée : {}", ip_str);
        }
    }

    Ok(())
}

// ── API publique : blocage/déblocage manuel depuis les routes HTTP ─────────────

/// Bloquer manuellement une IP (appelable depuis un futur plugin)
#[allow(dead_code)]
pub async fn block_ip(
    bpf:    &mut Bpf,
    pool:   &MySqlPool,
    ip_str: &str,
    reason: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ip: Ipv4Addr = ip_str.parse()?;
    let ip_net = u32::from(ip).to_be();
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos() as u64;

    let mut blacklist: BpfHashMap<_, u32, u64> = BpfHashMap::try_from(
        bpf.map_mut("BLACKLIST").ok_or("Map BLACKLIST introuvable")?
    )?;
    blacklist.insert(ip_net, now_ns, 0)?;

    let now = chrono::Local::now().naive_local();
    sqlx::query(
        "INSERT IGNORE INTO ebpf_blacklist (ip_address, blocked_at, reason)
         VALUES (?, ?, ?)"
    )
    .bind(ip_str)
    .bind(now)
    .bind(reason)
    .execute(pool)
    .await?;

    println!("[ebpf_worker] ✓ IP bloquée manuellement : {} ({})", ip_str, reason);
    Ok(())
}

/// Débloquer manuellement une IP
#[allow(dead_code)]
pub async fn unblock_ip(
    bpf:    &mut Bpf,
    pool:   &MySqlPool,
    ip_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ip: Ipv4Addr = ip_str.parse()?;
    let ip_net = u32::from(ip).to_be();

    let mut blacklist: BpfHashMap<_, u32, u64> = BpfHashMap::try_from(
        bpf.map_mut("BLACKLIST").ok_or("Map BLACKLIST introuvable")?
    )?;
    blacklist.remove(&ip_net).ok();

    let now = chrono::Local::now().naive_local();
    sqlx::query(
        "UPDATE ebpf_blacklist
         SET unblocked_at = ?
         WHERE ip_address = ? AND unblocked_at IS NULL"
    )
    .bind(now)
    .bind(ip_str)
    .execute(pool)
    .await?;

    println!("[ebpf_worker] ✓ IP débloquée : {}", ip_str);
    Ok(())
}
