use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{fs, io};

use flexstr::SharedStr;

use crate::var::Vars;
use crate::{CodeFragments, CodeGenError};

const BUF_SIZE: usize = u16::MAX as usize;

// *** FragmentItem ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FragmentItem {
    // Must be first so Serde uses this one always
    Fragment(SharedStr),
    FragmentListRef(SharedStr),
}

// *** Fragment Lists ***

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct FragmentLists(HashMap<SharedStr, Vec<FragmentItem>>);

impl FragmentLists {
    pub fn build(&self) -> Self {
        let mut lists = HashMap::with_capacity(self.0.len());

        for (key, fragments) in &self.0 {
            let mut new_fragments = Vec::with_capacity(fragments.len());

            for fragment in fragments {
                match fragment {
                    FragmentItem::Fragment(s) | FragmentItem::FragmentListRef(s) => {
                        // If it is also a key, that means it is a list reference
                        if self.0.contains_key(s) {
                            new_fragments.push(FragmentItem::FragmentListRef(s.clone()));
                        } else {
                            new_fragments.push(FragmentItem::Fragment(s.clone()));
                        }
                    }
                }
            }

            lists.insert(key.clone(), new_fragments);
        }

        Self(lists)
    }

    pub fn validate_code_fragments(&self, code: &CodeFragments) -> Result<(), CodeGenError> {
        let mut missing = Vec::new();

        // Loop over each fragment list searching for each item in the code fragments
        for fragments in self.0.values() {
            let v: Vec<_> = fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    FragmentItem::Fragment(name) if !code.contains_key(name) => Some(name.clone()),
                    _ => None,
                })
                .collect();

            // Store all missing fragments
            missing.extend(v);
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(CodeGenError::MissingFragments(missing))
        }
    }

    pub fn validate_file(&self, name: &SharedStr, f: &File) -> Result<(), CodeGenError> {
        // Ensure the file's fragment list exists
        if !self.0.contains_key(&f.fragment_list) {
            return Err(CodeGenError::MissingFragmentList(
                f.fragment_list.clone(),
                name.clone(),
            ));
        }

        let mut missing = Vec::new();

        'top: for exception in &f.fragment_list_exceptions {
            // If it is the name of a list, we can bypass the 2nd scan entirely
            if self.0.contains_key(exception) {
                continue;
            }

            // If it might be the name of an actual fragment we will need to scan them all
            for fragment_list in self.0.values() {
                // As soon as we find a match jump to looking for next exception
                if fragment_list.iter().any(|fragment| match fragment {
                    FragmentItem::Fragment(name) => name == exception,
                    _ => false,
                }) {
                    continue 'top;
                }
            }

            // If we didn't find as a list or via scan, it is missing
            missing.push(exception.clone());
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(CodeGenError::MissingFragmentListExceptions(
                missing,
                name.clone(),
            ))
        }
    }
}

// *** Config ***

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct Common {
    #[serde(default)]
    base_path: PathBuf,
    #[serde(default)]
    rustfmt_path: PathBuf,
    #[serde(default)]
    vars: Vars,
}

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct File {
    path: PathBuf,
    fragment_list: SharedStr,
    #[serde(default)]
    fragment_list_exceptions: Vec<SharedStr>,
    vars: Vars,
}

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    common: Common,
    fragment_lists: FragmentLists,
    files: HashMap<SharedStr, File>,
}

impl Config {
    /// Try to load the `Config` from the given TOML reader
    pub fn from_toml_reader(
        r: impl io::Read,
        code: &CodeFragments,
    ) -> Result<Config, CodeGenError> {
        let mut reader = io::BufReader::new(r);
        let mut buffer = String::with_capacity(BUF_SIZE);
        reader.read_to_string(&mut buffer)?;

        let mut config: Config = toml::from_str(&buffer)?;
        config.build_and_validate(code)?;
        Ok(config)
    }

    /// Try to load the `Config` from the given TOML file
    pub fn from_toml_file(
        cfg_name: impl AsRef<Path>,
        code: &CodeFragments,
    ) -> Result<Config, CodeGenError> {
        match fs::File::open(cfg_name) {
            // If the file exists, but it can't be deserialized then report that error
            Ok(f) => Ok(Self::from_toml_reader(f, code)?),
            // Report any other I/O errors
            Err(err) => Err(err.into()),
        }
    }

    fn build_and_validate(&mut self, code: &CodeFragments) -> Result<(), CodeGenError> {
        // Build and validate fragment lists against code fragments and files
        self.fragment_lists = self.fragment_lists.build();

        self.fragment_lists.validate_code_fragments(code)?;
        for (name, file) in &self.files {
            self.fragment_lists.validate_file(name, file)?;
        }

        Ok(())
    }

    #[inline]
    pub fn file_names(&self) -> Vec<&SharedStr> {
        self.files.keys().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::str::FromStr;

    use flexstr::{shared_str, SharedStr};
    use pretty_assertions::assert_eq;
    use proc_macro2::TokenStream;
    use quote::quote;

    use crate::config::{Common, Config, File, FragmentItem, FragmentLists};
    use crate::var::{CodeValue, VarItem, VarValue};
    use crate::{register_fragments, CodeFragment, CodeGenError, TokenVars};

    const CONFIG: &str = r#"
        [common]
        base_path = "src/"
        rustfmt_path = "rustfmt"
        
        [common.vars]
        product = "FlexStr"
        generate = true
        count = 5
        suffix = "$ident$Str"
        list = [ "FlexStr", true, 5, "$ident$Str" ]
                
        [fragment_lists]
        impl = [ "impl_struct", "impl_core_ref" ]
        impl_struct = [ "empty", "from_ref" ]
        
        [files.str]
        path = "strings/generated/std_str.rs"
        fragment_list = "impl"
        fragment_list_exceptions = [ "impl_core_ref" ]
        
        [files.str.vars]
        str_type = "str"
    "#;

    struct ImplCoreRef;

    impl CodeFragment for ImplCoreRef {
        fn generate(&self, _vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
            Ok(quote! {})
        }
    }

    struct Empty;

    impl CodeFragment for Empty {
        fn generate(&self, _vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
            Ok(quote! {})
        }
    }

    struct FromRef;

    impl CodeFragment for FromRef {
        fn generate(&self, _vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
            Ok(quote! {})
        }
    }

    fn common() -> Common {
        let mut vars = HashMap::new();

        let product = VarValue::String(shared_str!("FlexStr"));
        vars.insert(shared_str!("product"), VarItem::Single(product.clone()));

        let generate = VarValue::Bool(true);
        vars.insert(shared_str!("generate"), VarItem::Single(generate.clone()));

        let count = VarValue::Number(5);
        vars.insert(shared_str!("count"), VarItem::Single(count.clone()));

        let suffix = VarValue::CodeValue(CodeValue::from_str("$ident$Str").unwrap());
        vars.insert(shared_str!("suffix"), VarItem::Single(suffix.clone()));

        vars.insert(
            shared_str!("list"),
            VarItem::List(vec![product, generate, count, suffix]),
        );

        Common {
            base_path: PathBuf::from("src/"),
            rustfmt_path: PathBuf::from("rustfmt"),
            vars,
        }
    }

    fn fragment_lists() -> FragmentLists {
        use FragmentItem::*;

        let mut lists = HashMap::new();
        lists.insert(
            shared_str!("impl"),
            vec![
                FragmentListRef(shared_str!("impl_struct")),
                Fragment(shared_str!("impl_core_ref")),
            ],
        );
        lists.insert(
            shared_str!("impl_struct"),
            vec![
                Fragment(shared_str!("empty")),
                Fragment(shared_str!("from_ref")),
            ],
        );
        FragmentLists(lists)
    }

    fn files() -> HashMap<SharedStr, File> {
        let mut str_vars = HashMap::new();
        str_vars.insert(
            shared_str!("str_type"),
            VarItem::Single(VarValue::String(shared_str!("str"))),
        );

        let files_str = File {
            path: PathBuf::from("strings/generated/std_str.rs"),
            fragment_list: shared_str!("impl"),
            fragment_list_exceptions: vec![shared_str!("impl_core_ref")],
            vars: str_vars,
        };

        let mut files = HashMap::new();
        files.insert(shared_str!("str"), files_str);
        files
    }

    #[test]
    fn from_reader() {
        let code = register_fragments!(ImplCoreRef, Empty, FromRef);
        let actual = Config::from_toml_reader(CONFIG.as_bytes(), &code).unwrap();
        let expected = Config {
            common: common(),
            fragment_lists: fragment_lists(),
            files: files(),
        };

        assert_eq!(expected, actual);
    }
}
