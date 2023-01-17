#[macro_export]
macro_rules! string_table_enum {
    ($enum_name: ident, $($option:ident),*) => {
        #[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
        #[allow(non_camel_case_types)]
        enum $enum_name {
            $($option, )*
            Unknown
        }

        impl $enum_name {
            #[allow(unused)]
            fn from_str(s: &str) -> Self {
                match s {
                    $(
                        stringify!($option) => Self::$option,
                    )*
                    _ => Self::Unknown
                }
            }

            #[allow(unused)]
            fn to_str(&self) -> &str {
                match self {
                    $(
                        Self::$option => stringify!($option),
                    )*
                    Self::Unknown => "unknown",
                }
            }
        }

        impl Default for $enum_name {
            fn default() -> Self { Self::Unknown }
        }
    };
}

#[cfg(test)]
mod test {
    use serde::{Serialize, Deserialize};

    string_table_enum!(MyEnum, option1, option2);

    #[test]
    fn test_enum_creation() {
        let n = MyEnum::from_str("option1");
        assert_eq!(n, MyEnum::option1);
    }

    #[test]
    fn test_enum_unknown() {
        let n = MyEnum::from_str("i want sausages");
        assert_eq!(n, MyEnum::Unknown);
    }
}