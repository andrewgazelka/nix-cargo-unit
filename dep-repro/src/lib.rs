use my_macro::MyDerive;
use serde::{Serialize, Deserialize};

#[derive(MyDerive, Serialize, Deserialize)]
pub struct MyStruct {
    pub data: shared_lib::SharedData,
}

pub fn test_it() -> MyStruct {
    MyStruct {
        data: shared_lib::create_data("hello"),
    }
}
