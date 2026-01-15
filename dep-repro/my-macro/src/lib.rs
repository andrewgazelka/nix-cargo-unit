use proc_macro::TokenStream;

#[proc_macro_derive(MyDerive)]
pub fn my_derive(_input: TokenStream) -> TokenStream {
    // Use shared_lib at compile time
    let _data = shared_lib::create_data("from macro");
    TokenStream::new()
}
