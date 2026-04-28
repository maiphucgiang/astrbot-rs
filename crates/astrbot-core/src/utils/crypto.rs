use ring::digest;

/// Hash a string with SHA-256
pub fn sha256(input: &str) -> String {
    let hash = digest::digest(&digest::SHA256, input.as_bytes());
    hex::encode(hash.as_ref())
}

/// Generate a random token
pub fn random_token(length: usize) -> String {
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    let mut buf = vec![0u8; length];
    rng.fill(&mut buf).unwrap_or(());
    hex::encode(&buf)
}
