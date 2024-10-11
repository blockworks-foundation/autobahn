use proc_macro2::Span;
use syn::Ident;

pub fn string_to_ident(prefix: &str, name: &str) -> Ident {
    let mut denominator_const_name = String::from(prefix);
    denominator_const_name.push_str(name);
    Ident::new(denominator_const_name.as_str(), Span::call_site())
}
