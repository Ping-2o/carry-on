//! Live mDNS/DNS-SD backend for the §18.3 discovery contract, engine-side.
//!
//! Advertise + browse `_carryon._tcp` with the [`mdns_sd`] daemon (pure Rust, no
//! async runtime, SO_REUSEPORT — coexists with the platform responder on macOS).
//! The **contract stays here**: every received TXT record goes through
//! [`TxtRecord::from_pairs`] (fail-closed filter — malformed records are dropped,
//! foreign keys ignored), and only peers whose `pm` equals our
//! [`PROTOCOL_MAJOR`] are surfaced. A platform responder never decides what a
//! peer is.
//!
//! ## Foreground-first / battery (plan stage 4)
//!
//! A browser holds a socket and wakes on every announcement — it is created
//! only while a shell screen is visible and stopped on leave (both apps drive
//! it from screen appear/disappear; nothing here ever self-schedules).
//!
//! ## Cache with TTL (plan stage 4)
//!
//! Resolved peers live in a small cache stamped with their last sighting
//! ([`PEER_CACHE_TTL`]); `snapshot()` prunes entries not re-announced within
//! the TTL. Revalidation at CONNECT time is the dial itself: a failed TLS
//! handshake makes the caller drop the peer — discovery never vouches for a
//! stale address.
//!
//! ## Scope honesty (§2)
//!
//! Multicast cannot run on CI runners: the live-network test in this module is
//! `#[ignore]`d (owner-machine runs, see `docs/evidence/`). Everything else is
//! unit-tested headless.

use crate::discovery::{DiscoveredPeer, Discovery, TxtRecord, SERVICE_TYPE};
use crate::error::{NetError, Result};
use crate::wire::PROTOCOL_MAJOR;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a peer stays listed without being re-announced. Announcements
/// repeat well inside this window (default DNS-SD TTL is 120s+; responders
/// re-announce far more often), so a live peer never flickers out.
pub const PEER_CACHE_TTL: Duration = Duration::from_secs(30);

/// The full browse domain (`mdns-sd` requires the `.local.` suffix).
fn browse_domain() -> String {
    format!("{SERVICE_TYPE}.local.")
}

fn net_err(what: &str, e: mdns_sd::Error) -> NetError {
    NetError::Io(format!("discovery {what}: {e}"))
}

/// One instance's cache entry: the last fully validated view + when seen.
struct CacheEntry {
    peer: DiscoveredPeer,
    last_seen: Instant,
    /// Foreign TXT keys observed and ignored (for honest diagnostics only).
    ignored_keys: Vec<String>,
}

// ---------------------------------------------------------------------------
// Advertise
// ---------------------------------------------------------------------------

/// Engine-side mDNS advertiser for `_carryon._tcp` (§18.3). The TXT record is
/// the engine's own [`TxtRecord`] — construction cannot leak anything else.
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    /// The registered fullname (`instance._carryon._tcp.local.`) for unregister.
    fullname: String,
    domain: String,
}

impl MdnsAdvertiser {
    /// Advertise `instance_name` on `port` with the §18.3 minimal TXT. The
    /// instance name is the DNS-SD label (the pilots' `carryon-<8hex>` shape —
    /// NOT a user or device name, §18.3 minimality).
    pub fn start(instance_name: &str, port: u16, txt: &TxtRecord) -> Result<Self> {
        if instance_name.is_empty() || instance_name.len() > 64 {
            return Err(NetError::Io(format!(
                "discovery instance name must be 1..=64 chars, got {}",
                instance_name.len()
            )));
        }
        let daemon = ServiceDaemon::new().map_err(|e| net_err("daemon", e))?;
        // Addresses follow the host's interfaces (addr_auto): the daemon keeps
        // the record correct as networks change. Hostname is derived from the
        // instance name — unique per advertise session, not user-identifying.
        let pairs = txt.to_pairs();
        let info = ServiceInfo::new(
            &browse_domain(),
            instance_name,
            &format!("{instance_name}-host.local."),
            "0.0.0.0",
            port,
            &pairs[..],
        )
        .map_err(|e| net_err("service info", e))?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(|e| net_err("register", e))?;
        Ok(Self {
            daemon,
            fullname,
            domain: browse_domain(),
        })
    }

    /// The fullname this advertiser registered (diagnostics).
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Unregister (best-effort) and shut the daemon down. Foreground-first:
    /// the shell stops advertising when its screen leaves.
    pub fn stop(self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.stop_browse(&self.domain);
        let _ = self.daemon.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Browse
// ---------------------------------------------------------------------------

/// Engine-side mDNS browser for `_carryon._tcp` with a TTL cache. Events are
/// drained non-blockingly on every [`MdnsBrowser::snapshot`]; the responder
/// thread never blocks the caller.
pub struct MdnsBrowser {
    daemon: ServiceDaemon,
    receiver: mdns_sd::Receiver<ServiceEvent>,
    domain: String,
    cache: std::sync::Mutex<HashMap<String, CacheEntry>>,
    /// Peers dropped by the fail-closed filter (malformed TXT / wrong pm) —
    /// honest diagnostics for the UI/evidence, never peers.
    rejections: std::sync::Mutex<Vec<String>>,
}

impl MdnsBrowser {
    /// Start browsing (foreground-only: the shell owns the lifecycle).
    pub fn start() -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(|e| net_err("daemon", e))?;
        let receiver = daemon
            .browse(&browse_domain())
            .map_err(|e| net_err("browse", e))?;
        Ok(Self {
            daemon,
            receiver,
            domain: browse_domain(),
            cache: std::sync::Mutex::new(HashMap::new()),
            rejections: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Drain pending daemon events into the cache (non-blocking).
    fn drain(&self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let fullname = info.get_fullname().to_string();
                    // §18.3 receive filter — the engine, not the responder,
                    // decides what a peer is. Malformed TXT: drop the peer.
                    let pairs: Vec<(String, String)> = info
                        .get_properties()
                        .iter()
                        .map(|prop| (prop.key().to_string(), prop.val_str().to_string()))
                        .collect();
                    let (txt, ignored) = match TxtRecord::from_pairs(&pairs) {
                        Ok(t) => t,
                        Err(e) => {
                            let mut rej = self.rejections.lock().expect("rejections lock");
                            let why = format!("{fullname}: {e}");
                            if !rej.contains(&why) {
                                rej.push(why);
                            }
                            continue;
                        }
                    };
                    // §18.2 discipline at discovery time: a peer advertising a
                    // different protocol major is not interoperable — filtered
                    // out here, before any dial is attempted.
                    if txt.protocol_major != PROTOCOL_MAJOR {
                        let mut rej = self.rejections.lock().expect("rejections lock");
                        let why = format!(
                            "{fullname}: pm={} != our {PROTOCOL_MAJOR}",
                            txt.protocol_major
                        );
                        if !rej.contains(&why) {
                            rej.push(why);
                        }
                        continue;
                    }
                    // SRV target address: prefer IPv4 (the apps dial IPv4).
                    let addr = info
                        .get_addresses()
                        .iter()
                        .find_map(|a| match a.to_ip_addr() {
                            std::net::IpAddr::V4(v4) => Some(v4.to_string()),
                            std::net::IpAddr::V6(_) => None,
                        });
                    let Some(addr) = addr else {
                        let mut rej = self.rejections.lock().expect("rejections lock");
                        let why = format!("{fullname}: no resolved address yet");
                        if !rej.contains(&why) {
                            rej.push(why);
                        }
                        continue;
                    };
                    let peer = DiscoveredPeer {
                        instance_id: txt.instance_id.clone(),
                        address: format!("{addr}:{}", info.get_port()),
                        txt,
                    };
                    let mut cache = self.cache.lock().expect("cache lock");
                    cache.insert(
                        fullname,
                        CacheEntry {
                            peer,
                            last_seen: Instant::now(),
                            ignored_keys: ignored,
                        },
                    );
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    self.cache.lock().expect("cache lock").remove(&fullname);
                }
                _ => {} // SearchStarted/ServiceFound/SearchStopped: lifecycle only
            }
        }
    }

    /// Current peers with their DNS-SD fullnames (drained, TTL-pruned, sorted
    /// by instance id). The fullname is diagnostics/UX context — the peer
    /// identity stays the §18.3 `id` TXT value.
    pub fn entries(&self) -> Vec<(String, DiscoveredPeer)> {
        self.drain();
        let mut cache = self.cache.lock().expect("cache lock");
        cache.retain(|_, e| e.last_seen.elapsed() < PEER_CACHE_TTL);
        let mut v: Vec<_> = cache
            .iter()
            .map(|(name, e)| (name.clone(), e.peer.clone()))
            .collect();
        v.sort_by(|a, b| a.1.instance_id.cmp(&b.1.instance_id));
        v
    }

    /// Current peers: drained, TTL-pruned, sorted by instance id.
    pub fn snapshot(&self) -> Vec<DiscoveredPeer> {
        self.entries().into_iter().map(|(_, p)| p).collect()
    }

    /// Why peers were rejected by the fail-closed filter (honest diagnostics).
    pub fn rejections(&self) -> Vec<String> {
        self.drain();
        self.rejections.lock().expect("rejections lock").clone()
    }

    /// Foreign (non-§18.3) TXT keys observed per live peer — data the engine
    /// has no semantics for: counted for honest diagnostics, never interpreted.
    pub fn foreign_keys(&self) -> Vec<(String, Vec<String>)> {
        self.drain();
        let cache = self.cache.lock().expect("cache lock");
        let mut v: Vec<_> = cache
            .iter()
            .map(|(name, e)| (name.clone(), e.ignored_keys.clone()))
            .collect();
        v.sort();
        v
    }

    /// Stop browsing and shut the daemon down.
    pub fn stop(self) {
        let _ = self.daemon.stop_browse(&self.domain);
        let _ = self.daemon.shutdown();
    }
}

impl Discovery for MdnsBrowser {
    fn browse(&self) -> Vec<DiscoveredPeer> {
        self.snapshot()
    }
}

// ---------------------------------------------------------------------------
// Owner-machine live test (multicast cannot run on CI runners)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// LIVE NETWORK TEST — run on the owner machine only (multicast on a real
    /// LAN; CI runners cannot multicast and this would flake). Procedure:
    ///
    /// ```sh
    /// cargo test -p carryon-net -- --ignored mdns_live --nocapture
    /// ```
    ///
    /// Asserts: a self-advertised `_carryon._tcp` instance is discovered,
    /// passes the §18.3 receive filter, and carries the exact TXT. This is
    /// OBSERVED (LOCAL) evidence — one machine, one NIC; the physical
    /// mac⇄A04 cross-device run is recorded separately in docs/evidence/.
    #[test]
    #[ignore = "live multicast on a real LAN — owner machine only (CI runners cannot multicast)"]
    fn mdns_live_advertise_then_browse_finds_self() {
        let txt = TxtRecord {
            protocol_major: PROTOCOL_MAJOR,
            instance_id: format!("deadbeef{:04x}", std::process::id() & 0xFFFF),
            pairing_required: true,
            device_class: crate::discovery::DeviceClass::Desktop,
            connection_hint: "49152".into(),
        };
        let instance = format!("carryon-{}", txt.instance_id);
        let advertiser =
            MdnsAdvertiser::start(&instance, 49152, &txt).expect("advertise on the real LAN");
        assert!(advertiser.fullname().starts_with(&instance));

        let deadline = Instant::now() + Duration::from_secs(20);
        let browser = MdnsBrowser::start().expect("browse on the real LAN");
        loop {
            let peers = browser.snapshot();
            if let Some(p) = peers.iter().find(|p| p.instance_id == txt.instance_id) {
                assert_eq!(p.txt, txt, "discovered TXT must equal the advertised one");
                assert!(
                    p.address.ends_with(":49152"),
                    "resolved SRV port, got {}",
                    p.address
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "self-instance not discovered within 20 s; rejections: {:?}",
                browser.rejections()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        advertiser.stop();
        browser.stop();
    }
}
