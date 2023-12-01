
pub struct KrpcClient {
    url: Option<String>
}

impl KrpcClient {
    pub fn build() -> KrpcClient {
        return KrpcClient { url: None };
    }

    pub fn set_url(mut self, url: &str) -> KrpcClient {
        let _ = self.url.insert(url.to_string());
        return self;
    }

    pub async fn run(self) {
        
    }
}
