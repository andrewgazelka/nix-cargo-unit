pub mod hello {
    tonic::include_proto!("hello");
}

use hello::{HelloReply, HelloRequest};

fn main() {
    // Just verify the generated code compiles
    let request = HelloRequest {
        name: "World".to_string(),
    };

    let reply = HelloReply {
        message: format!("Hello, {}!", request.name),
    };

    println!("Request: {:?}", request.name);
    println!("Reply: {:?}", reply.message);
    println!("Tonic proto codegen works!");
}
