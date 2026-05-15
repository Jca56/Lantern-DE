//! Click-expand panel drawing, hit-testing, and layout for the WiFi
//! control. Pure-render code: all backend state lives on
//! [`super::Wifi`], which these modules only read from. (The inline
//! tile + shared signal icon live in `super::super::tile`.)

mod cards;
mod draw;
mod hit;
mod layout;

pub use draw::draw_view;
pub use hit::{hit_test_network, NetworkHit};
pub use layout::{max_scroll, row_list_top_y};

// ── Layout constants (logical px unless noted) ──────────────────────────────

pub(super) const VIEW_TOP_PAD: f32 = 24.0;
pub(super) const VIEW_HEADER_FONT: f32 = 26.0;
pub(super) const VIEW_HEADER_BOTTOM_GAP: f32 = 12.0;
pub(super) const ROW_HEIGHT: f32 = 64.0;
pub(super) const ROW_FONT: f32 = 26.0;
pub(super) const ROW_SIGNAL_SIZE: f32 = 32.0;
pub(super) const ROW_SIGNAL_GAP: f32 = 16.0;
pub(super) const ROW_LOCK_SIZE: f32 = 22.0;
pub(super) const ROW_RIGHT_GAP: f32 = 12.0;
/// Cap on rendered network rows. We scroll past this, so it's purely a
/// "don't try to lay out the entire 200-AP city scan" sanity bound.
pub(super) const MAX_NETWORK_ROWS: usize = 64;
/// Bottom padding the list reserves so the last row doesn't hug the
/// panel edge.
pub(super) const LIST_BOTTOM_PAD: f32 = 16.0;
/// Logical px reserved for the expanded detail+button area beneath
/// the row header. One detail line per displayed property.
pub(super) const EXPAND_PAD_TOP: f32 = 12.0;
pub(super) const EXPAND_PAD_BOTTOM: f32 = 14.0;
pub(super) const EXPAND_LINE_GAP: f32 = 8.0;
pub(super) const EXPAND_DETAIL_FONT: f32 = 19.0;
pub(super) const EXPAND_LABEL_W_FRAC: f32 = 0.28;
pub(super) const EXPAND_BUTTON_TOP_GAP: f32 = 14.0;
pub(super) const EXPAND_BUTTON_H: f32 = 44.0;
pub(super) const EXPAND_BUTTON_FONT: f32 = 22.0;
pub(super) const EXPAND_BUTTON_W: f32 = 160.0;
/// Band-selector pills sit between the details list and the Connect
/// button. Shown only when an SSID is advertised on multiple bands.
pub(super) const BAND_ROW_TOP_GAP: f32 = 14.0;
pub(super) const BAND_PILL_H: f32 = 36.0;
pub(super) const BAND_PILL_W: f32 = 72.0;
pub(super) const BAND_PILL_GAP: f32 = 8.0;
pub(super) const BAND_PILL_FONT: f32 = 18.0;
pub(super) const BAND_LABEL_FONT: f32 = 18.0;

/// "VPN: ON/OFF" pill on the right edge of the header row.
pub(super) const VPN_LABEL_FONT: f32 = 22.0;
/// Hit-zone padding around the VPN label so it's comfortable to click.
pub(super) const VPN_HIT_PAD_X: f32 = 8.0;
pub(super) const VPN_HIT_PAD_Y: f32 = 4.0;

/// Width fraction (of the expanded inner row) for the left column
/// (details + band pills + Connect button). The right column hosts
/// the top-BSSID cards.
pub(super) const LEFT_COL_FRAC: f32 = 0.58;
/// Column-gutter padding between left and right columns.
pub(super) const COL_GUTTER: f32 = 14.0;
/// BSSID card constants (right column).
pub(super) const BSSID_HEADER_FONT: f32 = 18.0;
pub(super) const BSSID_HEADER_BOTTOM_GAP: f32 = 6.0;
pub(super) const BSSID_CARD_H: f32 = 56.0;
pub(super) const BSSID_CARD_GAP: f32 = 6.0;
pub(super) const BSSID_MAC_FONT: f32 = 17.0;
pub(super) const BSSID_META_FONT: f32 = 14.0;
pub(super) const BSSID_LOCK_SIZE: f32 = 22.0;
pub(super) const BSSID_LOCK_PAD: f32 = 10.0;
pub(super) const MAX_BSSID_CARDS: usize = 5;

/// Saved-profile card constants.
pub(super) const PROFILE_SECTION_TOP_GAP: f32 = 12.0;
pub(super) const PROFILE_HEADER_FONT: f32 = 18.0;
pub(super) const PROFILE_HEADER_BOTTOM_GAP: f32 = 6.0;
pub(super) const PROFILE_CARD_H: f32 = 56.0;
pub(super) const PROFILE_CARD_GAP: f32 = 6.0;
pub(super) const PROFILE_NAME_FONT: f32 = 17.0;
pub(super) const PROFILE_META_FONT: f32 = 14.0;
pub(super) const PROFILE_DELETE_SIZE: f32 = 22.0;
pub(super) const PROFILE_DELETE_PAD: f32 = 10.0;
pub(super) const PROFILE_ACTIVE_DOT: f32 = 8.0;
pub(super) const MAX_PROFILE_CARDS: usize = 6;
