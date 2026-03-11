use std::collections::HashMap;
use std::path::PathBuf;

/// Simple credential store — saves passwords per-origin in an XOR-obfuscated JSON file.
/// Not military-grade, but keeps them off disk in plaintext.
/// In the future we can upgrade to libsecret/KWallet D-Bus.

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Credential {
    pub username: String,
    password_enc: Vec<u8>,
}

impl Credential {
    fn new(username: &str, password: &str) -> Self {
        Self {
            username: username.to_string(),
            password_enc: xor_bytes(password.as_bytes(), KEY),
        }
    }

    pub fn password(&self) -> String {
        String::from_utf8_lossy(&xor_bytes(&self.password_enc, KEY)).to_string()
    }
}

// Simple obfuscation key — prevents casual snooping, not a security boundary
const KEY: &[u8] = b"lantern-browser-key-2026";

fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

/// origin -> list of credentials (some sites have multiple accounts)
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct PasswordStore {
    entries: HashMap<String, Vec<Credential>>,
}

impl PasswordStore {
    pub fn load() -> Self {
        let path = store_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = store_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn store(&mut self, origin: &str, username: &str, password: &str) {
        let creds = self.entries.entry(origin.to_string()).or_default();
        // Update existing or add new
        if let Some(existing) = creds.iter_mut().find(|c| c.username == username) {
            *existing = Credential::new(username, password);
        } else {
            creds.push(Credential::new(username, password));
        }
        self.save();
    }

    pub fn lookup(&self, origin: &str) -> Option<&Vec<Credential>> {
        self.entries.get(origin)
    }
}

fn store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(format!("{}/.local/share/lntrn-browser/passwords.json", home))
}

/// JS to inject on page load — detects form submissions with password fields
pub const FORM_DETECT_JS: &str = r#"
(function() {
    if (window.__lntrn_pw_hooked) return;
    window.__lntrn_pw_hooked = true;

    document.addEventListener('submit', function(e) {
        var form = e.target;
        var pw = form.querySelector('input[type="password"]');
        if (!pw || !pw.value) return;

        // Find the username field — common patterns
        var user = form.querySelector(
            'input[type="email"], input[name="username"], input[name="email"], ' +
            'input[name="login"], input[name="user"], input[autocomplete="username"], ' +
            'input[type="text"]'
        );
        var username = user ? user.value : '';
        if (!username) return;

        // Send to browser via title hack (WebKitGTK doesn't have easy JS->Rust channels)
        var msg = JSON.stringify({
            __lntrn_save_pw: true,
            origin: window.location.origin,
            username: username,
            password: pw.value
        });
        window.postMessage({type: '__lntrn_pw', data: msg}, '*');

        // Use a custom event on document.title change as signal
        var oldTitle = document.title;
        document.title = '__LNTRN_PW__' + msg;
        setTimeout(function() { document.title = oldTitle; }, 50);
    }, true);
})();
"#;

/// JS to autofill credentials on a page
pub fn autofill_js(username: &str, password: &str) -> String {
    // Escape for JS string
    let u = username.replace('\\', "\\\\").replace('\'', "\\'");
    let p = password.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        r#"
(function() {{
    var pw = document.querySelector('input[type="password"]');
    if (!pw) return;
    var form = pw.closest('form') || document;
    var user = form.querySelector(
        'input[type="email"], input[name="username"], input[name="email"], ' +
        'input[name="login"], input[name="user"], input[autocomplete="username"], ' +
        'input[type="text"]'
    );
    if (user) {{
        user.value = '{u}';
        user.dispatchEvent(new Event('input', {{bubbles: true}}));
    }}
    pw.value = '{p}';
    pw.dispatchEvent(new Event('input', {{bubbles: true}}));
}})();
"#,
        u = u,
        p = p
    )
}
