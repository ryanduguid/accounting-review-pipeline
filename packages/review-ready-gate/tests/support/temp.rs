use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct TempDir(pub PathBuf);
impl TempDir {
    pub fn new() -> Self {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("reviewready-test-{name}"));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}
