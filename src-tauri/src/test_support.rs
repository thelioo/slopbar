use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "slopbar-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn slot(id: &str, access: &str, refresh: &str, expires_ms: i64) -> crate::accounts::Slot {
    crate::accounts::Slot {
        id: id.into(),
        provider: crate::accounts::Provider::Claude,
        email: Some("test@example.invalid".into()),
        plan: None,
        org: None,
        data: serde_json::json!({ "credentials": { "claudeAiOauth": {
            "accessToken": access, "refreshToken": refresh, "expiresAt": expires_ms,
        } } }),
    }
}
