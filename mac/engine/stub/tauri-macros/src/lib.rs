use proc_macro::TokenStream;

/// The engine helper has no webview, so a Tauri command is just a function.
#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
