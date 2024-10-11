use quote::quote;

use crate::utils::string_to_ident;
use crate::DecimalCharacteristics;

pub fn generate_ops(characteristics: DecimalCharacteristics) -> proc_macro::TokenStream {
    let DecimalCharacteristics {
        struct_name,
        underlying_type,
        ..
    } = characteristics;

    let name_str = &struct_name.to_string();
    let underlying_str = &underlying_type.to_string();

    let module_name = string_to_ident("tests_", &name_str);

    proc_macro::TokenStream::from(quote!(
        impl std::ops::Add for #struct_name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self::new(self.get()
                    .checked_add(rhs.get())
                    .unwrap_or_else(|| panic!("decimal: overflow in method {}::add()", #name_str))
                )
            }
        }

        impl std::ops::Sub for #struct_name {
            type Output = #struct_name;

            fn sub(self, rhs: Self) -> #struct_name {
                Self::new(self.get()
                    .checked_sub(rhs.get())
                    .unwrap_or_else(|| panic!("decimal: overflow in method {}::sub()", #name_str))
                )
            }
        }

        impl<T: Decimal> std::ops::Mul<T> for #struct_name
        where
            T::U: TryInto<#underlying_type>,
        {
            type Output = #struct_name;

            fn mul(self, rhs: T) -> Self {
                Self::new(
                    self.get()
                        .checked_mul(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::mul()", #underlying_str, #name_str))
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::mul()", #name_str))
                        .checked_div(T::one())
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::mul()", #name_str))
                )
            }
        }

        impl<T: Decimal> std::ops::Div<T> for #struct_name
        where
            T::U: TryInto<#underlying_type>,
        {
            type Output = Self;

            fn div(self, rhs: T) -> Self {
                Self::new(
                    self.get()
                        .checked_mul(T::one())
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::div()", #name_str))
                        .checked_div(
                            rhs.get()
                                .try_into()
                                .unwrap_or_else(|_| std::panic!("decimal: rhs value can't fit into `{}` type in {}::div()", #underlying_str, #name_str))
                        )
                        .unwrap_or_else(|| std::panic!("decimal: overflow in method {}::div()", #name_str))
                )
            }
        }

        impl std::ops::AddAssign for #struct_name {
            fn add_assign(&mut self, rhs: Self)  {
                *self = *self + rhs
            }
        }

        impl std::ops::SubAssign for #struct_name {
            fn sub_assign(&mut self, rhs: Self)  {
                *self = *self - rhs
            }
        }

        impl std::ops::MulAssign for #struct_name {
            fn mul_assign(&mut self, rhs: Self)  {
                *self = *self * rhs
            }
        }

        impl std::ops::DivAssign for #struct_name {
            fn div_assign(&mut self, rhs: Self)  {
                *self = *self / rhs
            }
        }


        #[cfg(test)]
        pub mod #module_name {
            use super::*;

            #[test]
            fn test_add () {
                let mut a = #struct_name::new(1);
                let b = #struct_name::new(1);
                assert_eq!(a + b, #struct_name::new(2));
                a += b;
                assert_eq!(a, #struct_name::new(2));
            }

            #[test]
            fn test_sub () {
                let mut a = #struct_name::new(1);
                let b = #struct_name::new(1);
                assert_eq!(a - b, #struct_name::new(0));
                a -= b;
                assert_eq!(a, #struct_name::new(0));
            }

            #[test]
            fn test_mul () {
                let mut a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a * b, #struct_name::new(2));
                a *= b;
                assert_eq!(a, #struct_name::new(2));
            }

            #[test]
            fn test_div () {
                let mut a = #struct_name::new(2);
                let b = #struct_name::new(#struct_name::one());
                assert_eq!(a / b, #struct_name::new(2));
                a /= b;
                assert_eq!(a, #struct_name::new(2));
            }
        }
    ))
}
