# ebpf-firewall — Programme XDP kernel pour small-folks

Filtre les paquets réseau **dans le kernel Linux** avant la stack TCP/IP.

## Prérequis

```bash
# Rust nightly + composant rust-src
rustup component add rust-src

# Linker BPF
cargo install bpf-linker

# Kernel ≥ 5.8 et droits root/CAP_BPF
```

## Compilation

```bash
cd ebpf-firewall
cargo build --release
# → target/bpfel-unknown-none/release/ebpf-firewall
```

## Activation dans small-folks

1. Décommenter dans `Cargo.toml` : `aya = { version = "0.12" }`
2. Décommenter dans `src/main.rs` : `mod ebpf_worker;` + bloc de démarrage
3. Configurer `.env` :
   ```
   EBPF_ENABLED=true
   EBPF_INTERFACE=eth0
   EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
   EBPF_RATE_LIMIT=100
   EBPF_WINDOW_SECS=60
   EBPF_AUTO_UNBLOCK_SECS=300
   ```
4. Lancer small-folks avec `sudo` ou `CAP_BPF`

## Routes HTTP disponibles (via plugin_sql)

```json
GET /security/blacklist   → liste des IPs bloquées
GET /api/security/stats   → statistiques XDP
```

À ajouter dans `config_actions.json` :

```json
"GET/security/blacklist": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ip_address, CAST(blocked_at AS CHAR) AS blocked_at, reason, CAST(unblock_at AS CHAR) AS unblock_at FROM ebpf_blacklist WHERE unblocked_at IS NULL ORDER BY blocked_at DESC",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
},
"GET/api/security/blacklist": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ip_address, CAST(blocked_at AS CHAR) AS blocked_at, reason FROM ebpf_blacklist WHERE unblocked_at IS NULL ORDER BY blocked_at DESC",
    "return_type": "json",
    "auth": true
}
```

## Architecture

```
Paquet réseau
  ↓ (NIC)
[xdp_firewall] ← tourne dans le kernel
  ├─ IP dans BLACKLIST ?  → XDP_DROP (< 100ns)
  ├─ Paquet SYN ?         → incrémenter CONN_COUNT[src_ip]
  │   └─ count > rate_limit ? → ajouter BLACKLIST → XDP_DROP
  └─ Sinon                → XDP_PASS
         ↕ BPF Maps
[ebpf_worker]  ← tourne en userspace (Tokio)
  ├─ Lit STATS  toutes les 5s → logs
  ├─ Sync BLACKLIST → MySQL (ebpf_blacklist)
  └─ Auto-débloque IPs expirées
```
