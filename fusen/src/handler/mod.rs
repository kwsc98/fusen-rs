use self::loadbalance::LoadBalance_;
pub mod loadbalance;

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
}

pub struct Handler {
    id: String,
    handler_invoker: HandlerInvoker,
}

impl Handler {
    pub fn new(id: String, handler_invoker: HandlerInvoker) -> Self {
        Self {
            id,
            handler_invoker,
        }
    }
}

pub trait HandlerLoad {
    fn load(self) -> Handler;
}
