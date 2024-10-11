use quote::quote;

use crate::DecimalCharacteristics;

pub fn generate_base(characteristics: DecimalCharacteristics) -> proc_macro::TokenStream {
    let DecimalCharacteristics {
        struct_name,
        underlying_type,
        scale: parsed_scale,
        field_name,
        ..
    } = characteristics;

    let denominator = 10u128.pow(parsed_scale as u32);
    let almost_denominator = denominator.checked_sub(1).unwrap();

    proc_macro::TokenStream::from(quote!(
        impl Decimal for #struct_name {
            type U = #underlying_type;

            fn get(&self) -> #underlying_type {
                self.#field_name
            }

            fn new(value: Self::U) -> Self {
                let mut created = #struct_name::default();
                created.#field_name = value;
                created
            }

            fn max_value() -> Self::U {
                Self::U::MAX
            }

            fn max_instance() -> Self {
                Self::new(Self::max_value())
            }

            fn here<T: TryFrom<Self::U>>(&self) -> T {
                match T::try_from(self.#field_name) {
                    Ok(v) => v,
                    Err(_) => std::panic!("could not parse {} to {}", "T", "u8"),
                }
            }

            fn scale() -> u8 {
                #parsed_scale
            }

            fn one<T: TryFrom<u128>>() -> T {
                match T::try_from(#denominator) {
                    Ok(v) => v,
                    Err(_) => std::panic!("denominator wouldn't fit into this type",),
                }
            }

            fn checked_one<T: TryFrom<u128>>() -> std::result::Result<T, String> where
                T::Error: std::fmt::Display,
            {
                T::try_from(#denominator).map_err(|err| format!("checked_one: can not get one to type {} : {}", std::any::type_name::<T>(), err.to_string()))
            }

            fn almost_one<T: TryFrom<u128>>() -> T {
                match T::try_from(#almost_denominator) {
                    Ok(v) => v,
                    Err(_) => std::panic!("denominator wouldn't fit into this type",),
                }
            }
        }
    ))
}
