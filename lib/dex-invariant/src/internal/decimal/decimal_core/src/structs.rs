use proc_macro2::TokenStream;
use syn::Ident;

#[derive(Debug, Clone)]
pub struct DecimalCharacteristics {
    pub struct_name: Ident,
    pub field_name: TokenStream, // cannot be Ident because of tuple structs
    pub underlying_type: Ident,
    pub big_type: Ident,
    pub scale: u8,
}
