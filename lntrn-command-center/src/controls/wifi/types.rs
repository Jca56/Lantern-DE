//! Plain data types shared by the WiFi state, worker, and view code.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiState {
    Off,
    Disconnected,
    Connected { ssid: String, signal: u32 },
}

/// Radio band a given AP is broadcasting on. Used to let the user pin
/// a connection to a specific band when a hotspot advertises the same
/// SSID on both 2.4 and 5 GHz radios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Band {
    G24,
    G5,
    G6,
}

impl Band {
    /// Short pill label, e.g. "2.4".
    pub fn short_label(self) -> &'static str {
        match self {
            Band::G24 => "2.4",
            Band::G5 => "5",
            Band::G6 => "6",
        }
    }

    /// Long-form label for the details panel, e.g. "2.4 GHz".
    pub fn long_label(self) -> &'static str {
        match self {
            Band::G24 => "2.4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }

    pub(crate) fn from_mhz(mhz: u32) -> Option<Band> {
        Some(match mhz {
            2400..=2500 => Band::G24,
            4900..=5900 => Band::G5,
            5925..=7125 => Band::G6,
            _ => return None,
        })
    }
}

/// One radio's worth of info for an SSID. A given network may have
/// multiple `BandEntry`s — typically one for 2.4 and one for 5 GHz when
/// a hotspot broadcasts both.
#[derive(Debug, Clone)]
pub struct BandEntry {
    pub band: Band,
    pub signal: u32,
    pub bssid: String,
    pub channel: String,
    pub frequency: String,
    pub rate: String,
}

#[derive(Debug, Clone)]
pub struct Network {
    pub ssid: String,
    /// Strongest signal across all bands — drives the row icon + sort.
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
    pub saved: bool,
    /// BSSID of the strongest band. Kept for the details panel; the
    /// per-band BSSIDs live in `bands`.
    pub bssid: String,
    /// e.g. "Infra" / "Mesh".
    pub mode: String,
    /// Strongest band's channel (string, as nmcli reports).
    pub channel: String,
    /// Strongest band's frequency, e.g. "5180 MHz".
    pub frequency: String,
    /// Strongest band's negotiated bitrate, e.g. "270 Mbit/s".
    pub rate: String,
    /// Saved NetworkManager profiles whose `802-11-wireless.ssid`
    /// matches this network. Multiple is the common case when the user
    /// has reconnected with different settings (Wi-Fi password change,
    /// a band/BSSID pin, etc.) — surfacing them in the UI gives them a
    /// way to clean up duplicates.
    pub profiles: Vec<Profile>,
    /// All radios this SSID is advertised on, sorted by signal desc.
    /// One entry per band, holding the strongest BSSID for that band —
    /// drives the band-selector pills.
    pub bands: Vec<BandEntry>,
    /// EVERY BSSID broadcasting this SSID (any band, any AP), sorted by
    /// signal desc. Used by the BSSID column in the expanded panel.
    pub aps: Vec<BandEntry>,
    /// Band the user has selected for connecting. Defaults to the
    /// strongest band; persists across rescans (see `Wifi::tick`).
    pub selected_band: Band,
    /// BSSID the user has pinned via the lock icon. When `Some`, future
    /// connect attempts will force this specific access point via
    /// `nmcli` `wifi.bssid=<mac>`. Persists in memory across rescans.
    pub pinned_bssid: Option<String>,
    /// Encryption flags (e.g. "WPA2 WPA3 PSK CCMP"). Built from
    /// SECURITY + WPA-FLAGS + RSN-FLAGS during the scan.
    pub flags_summary: String,
}

/// A saved NetworkManager profile. We attach a `Vec<Profile>` per
/// `Network` so the expanded panel can list duplicates / per-SSID
/// variations and offer delete + activate actions.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub uuid: String,
    /// `802-11-wireless.bssid` field on the profile, if any.
    pub pinned_bssid: Option<String>,
    /// `802-11-wireless.band` value ("bg", "a", or empty).
    pub pinned_band: Option<String>,
    /// Unix seconds — last time the profile was activated. iwd's
    /// `KnownNetwork.LastConnectedTime` populates this; zero when never
    /// activated. Not currently surfaced in the UI but kept for future
    /// "recent networks" ordering.
    #[allow(dead_code)]
    pub timestamp: i64,
    /// True when this profile is the currently-active connection.
    pub active: bool,
}
