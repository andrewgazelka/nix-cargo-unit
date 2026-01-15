use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct SharedData {
    pub value: String,
}

pub fn create_data(s: &str) -> SharedData {
    SharedData { value: s.to_string() }
}
