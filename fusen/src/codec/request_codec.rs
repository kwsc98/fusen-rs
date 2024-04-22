use std::marker::PhantomData;

use fusen_common::{register::Type, FusenContext};
use http::Request;

pub(crate) trait RequestCodec<T> {
    fn encode(&self, msg: FusenContext) -> Request<T>;

    fn decode(&self, request: Request<T>) -> FusenContext;
}

pub struct RequestHandler<T> {
    _t: PhantomData<T>,
}


impl<T> RequestCodec<T> for RequestHandler<T> {
    fn encode(&self, mut msg: FusenContext) -> Request<T> {
        let path = match &msg.server_tyep {
            &Type::SpringCloud => msg.path,
            _ => {
                let path = "/".to_owned() + msg.class_name.as_ref() + "/" + &msg.method_name;
                match msg.path {
                    fusen_common::Path::GET(_) => fusen_common::Path::GET(path),
                    fusen_common::Path::POST(_) => fusen_common::Path::POST(path),
                }
            }
        };
        let path = match msg.path {
            fusen_common::Path::GET(path) => get_path,
            fusen_common::Path::POST(path) => path,
        }
    }

    fn decode(&self, request: Request<T>) -> FusenContext {
        // if request.method().to_string().to_lowercase().contains("get") {
        //     self.get_handler.decode(request)
        // } else {
        //     self.post_handler.decode(request)
        // }
    }
}

fn get_path(mut path : String, fields : &Vec<String>, msg : &Vec<String> ) -> String {
    if fields.len() > 0 {
        path.push_str("?");
        for idx in 0..fields.len() {
            path.push_str(&fields[idx]);
            path.push_str("=");
            path.push_str(&msg[idx]);
            path.push_str("&");
        }
        path.remove(path.len() - 1);
    }
    path
}