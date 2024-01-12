pub trait TestInterface {
    async fn hello(&self, name: String) -> String;
}
