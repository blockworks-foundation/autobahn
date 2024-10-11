use quote::quote;

use crate::utils::string_to_ident;
use crate::DecimalCharacteristics;

pub fn generate_big_ops(characteristics: DecimalCharacteristics) -> proc_macro::TokenStream {
    let DecimalCharacteristics {
        struct_name,
        big_type,
        underlying_type,
        ..
    } = characteristics;

    let name_str = &struct_name.to_string();
    let underlying_str = &underlying_type.to_string();
    let big_str = &big_type.to_string();

    let module_name = string_to_ident("tests_big_ops_", &name_str);

    proc_macro::TokenStream::from(quote!(
        impl<T: Decimal> BigOps<T> for #struct_name
        where
            T::U: TryInto<#big_type>,
        {
            fn big_mul(self, rhs: T) -> Self {
                Self::new(
                    #big_type::try_from(self.get())
                        .unwrap_or_else(|_| std::panic!("decimal: lhs value can't fit into `{}` type in {}::big_mul()", #big_str, #name_str))
                        .checked_mul(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::big_mul()", #big_str, #name_str))
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_mul()", #name_str))
                        .checked_div(
                            T::one()
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_mul()", #name_str))
                        .try_into()
                        .unwrap_or_else(|_| std::panic!("decimal: overflow casting result to `{}` type in method {}::big_mul()", #underlying_str, #name_str))

                )
            }

            fn big_mul_up(self, rhs: T) -> Self {
                Self::new(
                    #big_type::try_from(self.get())
                        .unwrap_or_else(|_| std::panic!("decimal: lhs value can't fit into `{}` type in {}::big_mul_up()", #big_str, #name_str))
                        .checked_mul(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::big_mul_up()", #big_str, #name_str))
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_mul_up()", #name_str))
                        .checked_add(T::almost_one())
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_mul_up()", #name_str))
                        .checked_div(
                            T::one()
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_mul_up()", #name_str))
                        .try_into()
                        .unwrap_or_else(|_| std::panic!("decimal: overflow casting result to `{}` type in method {}::big_mul_up()", #underlying_str, #name_str))
                )
            }

            fn big_div(self, rhs: T) -> Self {
                Self::new(
                    #big_type::try_from(self.get())
                        .unwrap_or_else(|_| std::panic!("decimal: lhs value can't fit into `{}` type in {}::big_div()", #big_str, #name_str))
                        .checked_mul(
                            T::one()
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_div()", #name_str))
                        .checked_div(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::big_div()", #big_str, #name_str))
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_div()", #name_str))
                        .try_into()
                        .unwrap_or_else(|_| std::panic!("decimal: overflow casting result to `{}` type in method {}::big_div()", #underlying_str, #name_str))
                )
            }

            fn big_div_up(self, rhs: T) -> Self {
                Self::new(
                    #big_type::try_from(self.get())
                        .unwrap_or_else(|_| std::panic!("decimal: lhs value can't fit into `{}` type in {}::big_div_up()", #big_str, #name_str))
                        .checked_mul(
                            T::one()
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_div_up()", #name_str))
                        .checked_add(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::big_div_up()", #big_str, #name_str))
                                .checked_sub(#big_type::from(1u128)).unwrap()
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_div_up()", #name_str))
                        .checked_div(
                            rhs.get()
                                .try_into().unwrap_or_else(|_| std::panic!("rhs value could not be converted to big type in `big_div_up`")),
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::big_div_up()", #name_str))
                        .try_into()
                        .unwrap_or_else(|_| std::panic!("decimal: overflow casting result to `{}` type in method {}::big_div_up()", #underlying_str, #name_str))
                )
            }
        }

        #[cfg(test)]
        pub mod #module_name {
            use super::*;

            #[test]
            fn test_big_mul () {
                let a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a.big_mul(b), #struct_name::new(2));
            }

            #[test]
            fn test_big_mul_up () {
                let a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a.big_mul_up(b), #struct_name::new(2));
            }

            #[test]
            fn test_big_div () {
                let a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a.big_div(b), #struct_name::new(2));
            }

            #[test]
            fn test_big_div_up () {
                let a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a.big_div_up(b), #struct_name::new(2));
            }
        }
    ))
}
