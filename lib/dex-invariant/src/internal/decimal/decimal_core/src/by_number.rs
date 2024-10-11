use quote::quote;

use crate::utils::string_to_ident;
use crate::DecimalCharacteristics;

pub fn generate_by_number(characteristics: DecimalCharacteristics) -> proc_macro::TokenStream {
    let DecimalCharacteristics {
        struct_name,
        big_type,
        ..
    } = characteristics;

    let name_str = &struct_name.to_string();

    let module_name = string_to_ident("tests_by_number_", &name_str);

    proc_macro::TokenStream::from(quote!(
        impl ByNumber<#big_type> for #struct_name {
            fn big_div_by_number(self, rhs: #big_type) -> Self {
                Self::new(
                    #big_type::try_from(self.get()).unwrap()
                        .checked_mul(
                            Self::one()
                        ).unwrap()
                        .checked_div(rhs).unwrap()
                        .try_into().unwrap()
                )
            }

            fn checked_big_div_by_number(self, rhs: #big_type) -> std::result::Result<Self, String> {
                Ok(Self::new(
                    #big_type::try_from(self.get()).map_err(|_| "checked_big_div_by_number: can't convert self to big_type")?
                    .checked_mul(Self::checked_one()?).ok_or_else(|| "checked_big_div_by_number: (self * Self::one()) multiplication overflow")?
                    .checked_div(rhs).ok_or_else(|| "checked_big_div_by_number: ((self * Self::one()) / rhs) division overflow")?
                    .try_into().map_err(|_| "checked_big_div_by_number: can't convert to result")?
                ))
            }

            fn big_div_by_number_up(self, rhs: #big_type) -> Self {
                Self::new(
                    #big_type::try_from(self.get()).unwrap()
                        .checked_mul(
                            Self::one()
                        ).unwrap()
                        .checked_add(
                            rhs.checked_sub(#big_type::from(1u8)).unwrap()
                        ).unwrap()
                        .checked_div(rhs).unwrap()
                        .try_into().unwrap()
                )
            }

            fn checked_big_div_by_number_up(self, rhs: #big_type) -> std::result::Result<Self, String> {
                Ok(Self::new(
                    #big_type::try_from(self.get()).map_err(|_| "checked_big_div_by_number_up: can't convert self to big_type")?
                    .checked_mul(Self::checked_one()?).ok_or_else(|| "checked_big_div_by_number_up: (self * Self::one()) multiplication overflow")?
                    .checked_add(
                        rhs.checked_sub(#big_type::from(1u8)).ok_or_else(|| "checked_big_div_by_number_up: (rhs - 1) subtraction overflow")?
                    ).ok_or_else(|| "checked_big_div_by_number_up: ((self * Self::one()) + (rhs - 1)) addition overflow")?
                    .checked_div(rhs).ok_or_else(|| "checked_big_div_by_number_up: (((self * Self::one()) + (rhs - 1)) / rhs) division overflow")?
                    .try_into().map_err(|_| "checked_big_div_by_number_up: can't convert to result")?
                ))
            }
        }

        impl<T: Decimal> ToValue<T, #big_type> for #struct_name
        where
            T::U: TryInto<#big_type>,
        {

            fn big_mul_to_value(self, rhs: T) -> #big_type {
                #big_type::try_from(self.get()).unwrap()
                    .checked_mul(
                        rhs.get()
                            .try_into().unwrap_or_else(|_| std::panic!("rhs value could not be converted to big type in `big_mul`")),
                    ).unwrap()
                    .checked_div(
                        T::one()
                    ).unwrap()
            }

            fn big_mul_to_value_up(self, rhs: T) -> #big_type {
                #big_type::try_from(self.get()).unwrap()
                    .checked_mul(
                        rhs.get()
                            .try_into().unwrap_or_else(|_| std::panic!("rhs value could not be converted to big type in `big_mul_up`")),
                    ).unwrap()
                    .checked_add(T::almost_one()).unwrap()
                    .checked_div(
                        T::one()
                    ).unwrap()
            }
        }

        #[cfg(test)]
        pub mod #module_name {
            use super::*;

            #[test]
            fn test_big_div_up_by_number () {
                let a = #struct_name::new(2);
                let b: #big_type = #struct_name::one();
                assert_eq!(a.big_div_by_number(b), #struct_name::new(2));
                assert_eq!(a.big_div_by_number_up(b), #struct_name::new(2));
            }

            #[test]
            fn test_checked_big_div_by_number() {
                let a = #struct_name::new(2);
                let b: #big_type = #struct_name::one();
                assert_eq!(a.checked_big_div_by_number(b), Ok(#struct_name::new(2)));
            }

            #[test]
            fn checked_big_div_by_number_up() {
                let a = #struct_name::new(2);
                let b: #big_type = #struct_name::one();
                assert_eq!(a.checked_big_div_by_number_up(b), Ok(#struct_name::new(2)));
            }

            #[test]
            fn test_big_mul_to_value () {
                let a = #struct_name::new(2);
                let b = #struct_name::from_integer(1);
                assert_eq!(a.big_mul_to_value(b), #big_type::from(a.get()));
                assert_eq!(a.big_mul_to_value_up(b), #big_type::from(a.get()));
            }
        }
    ))
}
