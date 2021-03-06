use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{fs, io};

use flexstr::SharedStr;

use crate::var::Vars;
use crate::{CodeFragments, Error, TokenVars};

const BUF_SIZE: usize = u16::MAX as usize;

const DEFAULT_FILENAME: &str = "flexgen.toml";

// *** FragmentItem ***

/// An enum that is either a reference to a code fragment or a fragment list
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FragmentItem {
    // Must be first so Serde uses this one always
    /// A single code fragment
    Fragment(SharedStr),
    /// A reference to a list of code fragments
    FragmentListRef(SharedStr),
}

// *** Fragment Lists ***

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
struct FragmentLists(HashMap<SharedStr, Vec<FragmentItem>>);

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

    pub fn validate_code_fragments(&self, code: &CodeFragments) -> Result<(), Error> {
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
            Err(Error::MissingFragments(missing))
        }
    }

    pub fn validate_file(&self, name: &SharedStr, f: &File) -> Result<(), Error> {
        // Ensure the file's fragment list exists
        if !self.0.contains_key(&f.fragment_list) {
            return Err(Error::MissingFragmentList(
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
            Err(Error::MissingFragmentListExceptions(missing, name.clone()))
        }
    }

    #[inline]
    pub fn fragment_list(&self, name: &SharedStr) -> Result<&Vec<FragmentItem>, Error> {
        self.0
            .get(name)
            .ok_or_else(|| Error::FragmentListNotFound(name.clone()))
    }
}

// *** Config ***

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
struct General {
    #[serde(default)]
    base_path: PathBuf,
    #[serde(default)]
    rust_fmt: RustFmt,
    #[serde(default)]
    vars: Vars,
}

impl General {
    #[inline]
    fn build_rust_fmt(&self) -> Option<rust_format::RustFmt> {
        self.rust_fmt.build_rust_fmt()
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
struct RustFmt {
    #[serde(default)]
    omit_final_format: bool,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    options: HashMap<SharedStr, SharedStr>,
}

impl RustFmt {
    fn build_rust_fmt(&self) -> Option<rust_format::RustFmt> {
        if !self.omit_final_format {
            let mut config = if !self.options.is_empty() {
                let map = self.options.iter().map(|(k, v)| (&**k, &**v)).collect();
                rust_format::Config::from_hash_map(map)
            } else {
                rust_format::Config::new()
            };
            if let Some(path) = &self.path {
                config = config.rust_fmt_path(path.clone())
            }

            Some(rust_format::RustFmt::from_config(config))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
struct File {
    path: PathBuf,
    fragment_list: SharedStr,
    #[serde(default)]
    fragment_list_exceptions: Vec<SharedStr>,
    vars: Vars,
}

/// The `flexgen` configuration
#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    general: General,
    fragment_lists: FragmentLists,
    files: HashMap<SharedStr, File>,
}

impl Config {
    /// Try to load the `Config` from the given TOML reader
    pub fn from_toml_reader(r: impl io::Read) -> Result<Config, Error> {
        let mut reader = io::BufReader::new(r);
        let mut buffer = String::with_capacity(BUF_SIZE);
        reader.read_to_string(&mut buffer)?;

        Ok(toml::from_str(&buffer)?)
    }

    /// Try to load the `Config` from the default TOML file (flexgen.toml)
    pub fn from_default_toml_file() -> Result<Config, Error> {
        let f = fs::File::open(DEFAULT_FILENAME)?;
        Self::from_toml_reader(f)
    }

    /// Try to load the `Config` from the given TOML file
    pub fn from_toml_file(cfg_name: impl AsRef<Path>) -> Result<Config, Error> {
        let f = fs::File::open(cfg_name)?;
        Self::from_toml_reader(f)
    }

    pub(crate) fn build_and_validate(&mut self, code: &CodeFragments) -> Result<(), Error> {
        // Build and validate fragment lists against code fragments and files
        self.fragment_lists = self.fragment_lists.build();

        self.fragment_lists.validate_code_fragments(code)?;
        for (name, file) in &self.files {
            self.fragment_lists.validate_file(name, file)?;
        }

        Ok(())
    }

    /// Return all the files names specified in the config
    #[inline]
    pub fn file_names(&self) -> Vec<&SharedStr> {
        self.files.keys().collect()
    }

    /// Return the specified file configuration
    #[inline]
    fn file(&self, name: &SharedStr) -> Result<&File, Error> {
        self.files
            .get(name)
            .ok_or_else(|| Error::FileNotFound(name.clone()))
    }

    /// Build the full file path to the file given as a parameter
    pub fn file_path(&self, name: &SharedStr) -> Result<PathBuf, Error> {
        let file = self.file(name)?;
        let base_path = self.general.base_path.as_os_str();

        let mut path = PathBuf::with_capacity(base_path.len() + file.path.as_os_str().len());
        path.push(base_path);
        path.push(&file.path);
        Ok(path)
    }

    #[inline]
    fn convert_vars(vars: &Vars) -> Result<TokenVars, Error> {
        vars.iter()
            .map(|(key, value)| match value.to_token_item() {
                Ok(value) => Ok((key.clone(), value)),
                Err(err) => Err(err),
            })
            .collect()
    }

    #[inline]
    fn general_vars(&self) -> Result<TokenVars, Error> {
        Self::convert_vars(&self.general.vars)
    }

    #[inline]
    fn file_vars(&self, name: &SharedStr) -> Result<TokenVars, Error> {
        Self::convert_vars(&self.file(name)?.vars)
    }

    /// Return the complete vars for the file name given as a parameter
    #[inline]
    pub fn vars(&self, name: &SharedStr) -> Result<TokenVars, Error> {
        let mut vars = self.general_vars()?;
        vars.extend(self.file_vars(name)?);
        Ok(vars)
    }

    /// Return the given named fragment list
    #[inline]
    pub fn fragment_list(&self, name: &SharedStr) -> Result<&Vec<FragmentItem>, Error> {
        self.fragment_lists.fragment_list(name)
    }

    /// Return the fragment list used by the file given a parameter
    #[inline]
    pub fn file_fragment_list(&self, name: &SharedStr) -> Result<&Vec<FragmentItem>, Error> {
        let name = &self.file(name)?.fragment_list;
        self.fragment_list(name)
    }

    /// Return all the fragment exceptions for the given file
    #[inline]
    pub fn file_fragment_exceptions(&self, name: &SharedStr) -> Result<&Vec<SharedStr>, Error> {
        Ok(&self.file(name)?.fragment_list_exceptions)
    }

    /// Return a [RustFmt](rust_format::RustFmt) instance configured as specified in this configuration
    #[inline]
    pub fn build_rust_fmt(&self) -> Option<rust_format::RustFmt> {
        self.general.build_rust_fmt()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::str::FromStr;

    use flexstr::{shared_str, SharedStr};
    use pretty_assertions::assert_eq;

    use crate::config::{Config, File, FragmentItem, FragmentLists, General, RustFmt};
    use crate::var::{CodeValue, VarItem, VarValue};

    const CONFIG: &str = r#"
        [general]
        base_path = "src/"
        
        [general.rust_fmt]
        path = "rustfmt"
        
        [general.vars]
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

    fn general() -> General {
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

        let rust_fmt = RustFmt {
            omit_final_format: false,
            path: Some("rustfmt".into()),
            options: Default::default(),
        };

        General {
            base_path: PathBuf::from("src/"),
            rust_fmt,
            vars,
        }
    }

    fn fragment_lists() -> FragmentLists {
        use FragmentItem::*;

        let mut lists = HashMap::new();
        lists.insert(
            shared_str!("impl"),
            vec![
                Fragment(shared_str!("impl_struct")),
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
        let actual = Config::from_toml_reader(CONFIG.as_bytes()).unwrap();
        let expected = Config {
            general: general(),
            fragment_lists: fragment_lists(),
            files: files(),
        };

        assert_eq!(expected, actual);
    }
}
