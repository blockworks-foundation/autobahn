#[macro_export]
macro_rules! size {
    ($name: ident) => {
        impl $name {
            pub const LEN: usize = std::mem::size_of::<$name>() + 8;
        }
    };
}
