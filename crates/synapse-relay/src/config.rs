pub struct Config;

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Ok(Config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
