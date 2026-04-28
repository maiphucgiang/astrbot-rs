pub trait PlatformAdapter: Send + Sync {
    fn name(&self) -> &str;
}
