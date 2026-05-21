//! Programme XDP eBPF — tourne dans le kernel Linux
//!
//! Ce code est compilé pour la cible `bpfel-unknown-none` (VM BPF du kernel).
//! Il s'exécute sur chaque paquet entrant sur l'interface réseau,
//! AVANT que la stack TCP/IP du kernel ne le traite.
//!
//! ## Logique de filtrage
//!
//! Pour chaque paquet IPv4/TCP entrant :
//!   1. Vérifier si l'IP source est dans BLACKLIST  → XDP_DROP immédiat
//!   2. Détecter les paquets SYN (nouvelles connexions)
//!   3. Incrémenter le compteur par IP dans CONN_COUNT
//!   4. Si compteur > seuil → ajouter à BLACKLIST → XDP_DROP
//!   5. Sinon → XDP_PASS (traitement normal)
//!
//! ## Maps eBPF partagées avec l'userspace
//!
//! - `CONN_COUNT`  : HashMap<u32 (IPv4), ConnEntry>  compteurs par IP
//! - `BLACKLIST`   : HashMap<u32 (IPv4), u64 (timestamp)> IPs bloquées
//! - `CONFIG`      : Array<u64> [rate_limit, window_ns]
//! - `STATS`       : Array<u64> [paquets_vus, drops_total, syn_total]

#![no_std]
#![no_main]
#![allow(static_mut_refs)]  // ← ajouter cette ligne

use aya_ebpf::{
    bindings::xdp_action,
    helpers::bpf_ktime_get_ns,
    macros::{map, xdp},
    maps::{Array, HashMap},
    programs::XdpContext,
};
use core::mem;

// ── Constantes protocole ──────────────────────────────────────────────────────
const ETH_P_IP:    u16 = 0x0800;
const IPPROTO_TCP: u8  = 6;
const TCP_SYN:     u8  = 0x02;
const ETH_HDR_LEN: usize = 14;

// ── Structures réseau ─────────────────────────────────────────────────────────
#[repr(C)]
struct EthHdr {
    dst_mac:    [u8; 6],
    src_mac:    [u8; 6],
    ether_type: u16,
}

#[repr(C)]
struct IpHdr {
    version_ihl: u8,
    tos:         u8,
    tot_len:     u16,
    id:          u16,
    frag_off:    u16,
    ttl:         u8,
    protocol:    u8,
    check:       u16,
    src_addr:    u32,
    dst_addr:    u32,
}

#[repr(C)]
struct TcpHdr {
    source:  u16,
    dest:    u16,
    seq:     u32,
    ack_seq: u32,
    flags:   u16,
    window:  u16,
    check:   u16,
    urg_ptr: u16,
}

// ── Maps eBPF ─────────────────────────────────────────────────────────────────
// Solution : deux maps séparées avec types primitifs (u32, u64)
// au lieu d'un type custom ConnEntry — évite le problème Pod

/// Compteur de SYN par IP source
#[map(name = "CONN_COUNT")]
static mut CONN_COUNT: HashMap<u32, u32> =
    HashMap::with_max_entries(65536, 0);

/// Timestamp du premier SYN par IP source (nanosecondes)
#[map(name = "CONN_FIRST")]
static mut CONN_FIRST: HashMap<u32, u64> =
    HashMap::with_max_entries(65536, 0);

/// Blacklist : IP → timestamp du blocage (ns)
#[map(name = "BLACKLIST")]
static mut BLACKLIST: HashMap<u32, u64> =
    HashMap::with_max_entries(4096, 0);

/// CONFIG[0] = rate_limit, CONFIG[1] = window_ns
#[map(name = "CONFIG")]
static mut CONFIG: Array<u64> =
    Array::with_max_entries(2, 0);

/// STATS[0] = paquets vus, STATS[1] = drops, STATS[2] = SYN
#[map(name = "STATS")]
static mut STATS: Array<u64> =
    Array::with_max_entries(3, 0);

// ── Point d'entrée XDP ────────────────────────────────────────────────────────
#[xdp]
pub fn xdp_firewall(ctx: XdpContext) -> u32 {
    match try_firewall(&ctx) {
        Ok(action) => action,
        Err(_)     => xdp_action::XDP_PASS,
    }
}

fn try_firewall(ctx: &XdpContext) -> Result<u32, ()> {
    increment_stat(0); // compteur paquets vus

    // ── Ethernet → IPv4 ? ─────────────────────────────────────────────────────
    let eth = ptr_at::<EthHdr>(ctx, 0)?;
    if u16::from_be(unsafe { (*eth).ether_type }) != ETH_P_IP {
        return Ok(xdp_action::XDP_PASS);
    }

    // ── IP header ─────────────────────────────────────────────────────────────
    let ip      = ptr_at::<IpHdr>(ctx, ETH_HDR_LEN)?;
    let src_ip  = unsafe { (*ip).src_addr };
    let protocol = unsafe { (*ip).protocol };

    // ── Blacklist check (toute IP, tout protocole) ────────────────────────────
    if unsafe { BLACKLIST.get(&src_ip) }.is_some() {
        increment_stat(1);
        return Ok(xdp_action::XDP_DROP);
    }

    if protocol != IPPROTO_TCP {
        return Ok(xdp_action::XDP_PASS);
    }

    // ── TCP header ────────────────────────────────────────────────────────────
    let ihl        = (unsafe { (*ip).version_ihl } & 0x0F) as usize * 4;
    let tcp_offset = ETH_HDR_LEN + ihl;
    let tcp        = ptr_at::<TcpHdr>(ctx, tcp_offset)?;
    let flags      = (u16::from_be(unsafe { (*tcp).flags }) & 0x01FF) as u8;

    // On ne compte que les SYN (nouvelles connexions)
    if flags & TCP_SYN == 0 {
        return Ok(xdp_action::XDP_PASS);
    }

    increment_stat(2); // compteur SYN

    let now_ns     = unsafe { bpf_ktime_get_ns() };
    let rate_limit = unsafe { CONFIG.get(0) }.copied().unwrap_or(100) as u32;
    let window_ns  = unsafe { CONFIG.get(1) }.copied().unwrap_or(60_000_000_000_u64);

    // ── Mettre à jour CONN_COUNT et CONN_FIRST ────────────────────────────────
    let should_block = if let Some(count_ptr) = unsafe { CONN_COUNT.get_ptr_mut(&src_ip) } {
        let count = unsafe { &mut *count_ptr };
        // Vérifier si on est dans la fenêtre de temps
        let first = unsafe { CONN_FIRST.get(&src_ip) }.copied().unwrap_or(now_ns);
        if now_ns.saturating_sub(first) > window_ns {
            // Hors fenêtre → réinitialiser
            *count = 1;
            unsafe { CONN_FIRST.insert(&src_ip, &now_ns, 0).ok() };
            false
        } else {
            *count += 1;
            *count > rate_limit
        }
    } else {
        // Première connexion depuis cette IP
        unsafe { CONN_COUNT.insert(&src_ip, &1u32, 0).ok() };
        unsafe { CONN_FIRST.insert(&src_ip, &now_ns, 0).ok() };
        false
    };

    // ── Blacklister si seuil dépassé ─────────────────────────────────────────
    if should_block {
        unsafe { BLACKLIST.insert(&src_ip, &now_ns, 0).ok() };
        increment_stat(1);
        return Ok(xdp_action::XDP_DROP);
    }

    Ok(xdp_action::XDP_PASS)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[inline(always)]
fn ptr_at<T>(ctx: &XdpContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end   = ctx.data_end();
    let size  = mem::size_of::<T>();
    if start + offset + size > end { return Err(()); }
    Ok((start + offset) as *const T)
}

#[inline(always)]
fn increment_stat(idx: u32) {
    if let Some(val) = unsafe { STATS.get_ptr_mut(idx) } {
        unsafe { *val = (*val).saturating_add(1) };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}