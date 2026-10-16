#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub bind: String,
    pub device: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 1092,
            bind: "127.0.0.1".to_string(),
            device: String::new(),
        }
    }
}
