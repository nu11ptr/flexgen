//! A simple crate to build source code use sections by combining multiple use section inputs
//! ```rust
//! use assert_unordered::assert_eq_unordered;
//! use quote::quote;
//! use use_builder::{UseBuilder, UseItems};
//!
//! // #1 - Build a two or more use trees and convert into `UseItems` (wrapped `Vec<ItemUse>`)
//!
//! let use1 = quote! {
//!     use crate::Test;
//!     use std::error::{Error as StdError};
//!     use std::fmt::Debug;
//! };
//!
//! let use2 = quote! {
//!     use syn::ItemUse;
//!     use std::fmt::Display;
//!     use crate::*;
//! };
//!
//! let items1: UseItems = syn::parse2(use1).unwrap();
//! let items2: UseItems = syn::parse2(use2).unwrap();
//!
//! // #2 - Parse, process, and extract into sections
//!
//! let builder = UseBuilder::from_uses(vec![items1, items2]);
//! let (std_use, ext_use, crate_use) = builder.into_items_sections().unwrap();
//!
//! // #3 - Validate our response matches expectation
//!
//! let std_expected = quote! {
//!     use std::error::Error as StdError;
//!     use std::fmt::{Debug, Display};
//! };
//! let std_expected = syn::parse2::<UseItems>(std_expected).unwrap().into_inner();
//!
//! let ext_expected = quote! {
//!     use syn::ItemUse;
//! };
//! let ext_expected = syn::parse2::<UseItems>(ext_expected).unwrap().into_inner();
//!
//! let crate_expected = quote! {
//!     use crate::*;
//! };
//! let crate_expected = syn::parse2::<UseItems>(crate_expected).unwrap().into_inner();
//!
//! assert_eq_unordered!(std_expected, std_use);
//! assert_eq_unordered!(ext_expected, ext_use);
//! assert_eq_unordered!(crate_expected, crate_use);
//! ```

#![warn(missing_docs)]

use quote::__private::TokenStream;
use quote::{ToTokens, TokenStreamExt};

use indexmap::IndexMap;
use std::collections::HashSet;
use std::error::Error as StdError;
use std::{cmp, fmt, hash};

const STD: [&str; 5] = ["std", "alloc", "core", "proc_macro", "test"];
const CRATE: [&str; 3] = ["self", "super", "crate"];

// *** UseItems ***

/// An opaque type primarily used for parsing to get an inner `Vec<syn::ItemUse>` (however,
/// [from_items](UseItems::from_items) can also be used for an existing [Vec] of items if parsing is
/// not required). This type is the sole input into [UseBuilder].
pub struct UseItems {
    items: Vec<syn::ItemUse>,
}

impl UseItems {
    /// Instead of using syn parsing, this can be used to wrap an existing [Vec] of use items
    #[inline]
    pub fn from_items(items: Vec<syn::ItemUse>) -> Self {
        Self { items }
    }

    /// Consume this value and emit the inner [Vec] of [syn::ItemUse]
    #[inline]
    pub fn into_inner(self) -> Vec<syn::ItemUse> {
        self.items
    }
}

impl syn::parse::Parse for UseItems {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Random guess on capacity
        let mut items = Vec::with_capacity(5);

        while !input.is_empty() {
            items.push(input.parse()?);
        }

        Ok(Self { items })
    }
}

impl ToTokens for UseItems {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(&self.items);
    }
}

impl IntoIterator for UseItems {
    type Item = syn::ItemUse;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

// *** Use Entry ***

#[derive(Clone, Debug, cmp::Eq, hash::Hash, cmp::PartialEq)]
enum UseKey {
    Name(syn::Ident),
    Rename(syn::Ident, syn::Ident),
    Glob,
}

// *** Use Data ***

#[derive(Clone, Debug, cmp::Eq, hash::Hash, cmp::PartialEq)]
struct UseData {
    vis: syn::Visibility,
    attrs: Vec<syn::Attribute>,
    has_leading_colons: bool,
}

impl UseData {
    #[inline]
    fn new(vis: syn::Visibility, attrs: Vec<syn::Attribute>, has_leading_colons: bool) -> Self {
        Self {
            vis,
            attrs,
            has_leading_colons,
        }
    }
}

// *** UseValue ***

#[derive(Clone, Default, Debug)]
struct UseValue {
    nodes: HashSet<UseData>,
    paths: UseBuilder,
}

// *** Error ***

/// The error type returned if issues occur during [UseBuilder] operations
#[derive(fmt::Debug)]
pub enum Error {
    /// A glob was found as the first entry in a use path - this is illegal Rust
    TopLevelGlob,
    /// A group was found as the first entry in a use path - this is not supported
    TopLevelGroup,
    /// The same use was found but with differing dual colon prefix, attributes, or visibility
    UseWithDiffAttr,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;

        match self {
            TopLevelGlob => f.write_str("Top level glob is not allowed"),
            TopLevelGroup => f.write_str("Top level group is not allowed"),
            UseWithDiffAttr => f.write_str(
                "Multiple copies of the same import with differing attributes are not allowed",
            ),
        }
    }
}

impl StdError for Error {}

// *** ItemUse Builder ***

#[derive(Clone, Default)]
struct ItemUseBuilder {
    paths: Vec<syn::Ident>,
}

impl ItemUseBuilder {
    #[inline]
    fn add_path(&mut self, path: syn::Ident) {
        self.paths.push(path);
    }

    fn into_item_use(mut self, names: Vec<UseKey>, data: UseData) -> syn::ItemUse {
        let key_to_tree = |key| match key {
            UseKey::Name(name) => syn::UseTree::Name(syn::UseName { ident: name }),
            UseKey::Rename(name, rename) => syn::UseTree::Rename(syn::UseRename {
                ident: name,
                as_token: Default::default(),
                rename,
            }),
            _ => unreachable!("Impossible glob"),
        };

        // #1 - Setup name tree

        // Regardless of number of entries in names, if there is a glob, ignore the rest
        let mut tree = if names.contains(&UseKey::Glob) {
            syn::UseTree::Glob(syn::UseGlob {
                star_token: Default::default(),
            })
        // If a single entry then it is either a name or rename
        } else if names.len() == 1 {
            // Panic safety: we verified there is exactly one item in the set
            key_to_tree(names.into_iter().next().unwrap())
        // Group
        } else {
            let items = names.into_iter().map(key_to_tree).collect();

            syn::UseTree::Group(syn::UseGroup {
                brace_token: Default::default(),
                items,
            })
        };

        // #2 - Build path (in reverse order)

        while !self.paths.is_empty() {
            let path = self.paths.remove(self.paths.len() - 1);

            tree = syn::UseTree::Path(syn::UsePath {
                ident: path,
                colon2_token: Default::default(),
                tree: Box::new(tree),
            });
        }

        // #3 - Build ItemUse

        let leading_colon = if data.has_leading_colons {
            Some(syn::token::Colon2::default())
        } else {
            None
        };

        syn::ItemUse {
            attrs: data.attrs,
            vis: data.vis,
            use_token: Default::default(),
            leading_colon,
            tree,
            semi_token: Default::default(),
        }
    }
}

// *** UseMap ***

/// Type that contains a partitioned list of uses by std, external, and crate level
pub type StdExtCrateUse = (Vec<syn::ItemUse>, Vec<syn::ItemUse>, Vec<syn::ItemUse>);

/// A type that builds vecs of [syn::ItemUse]. It takes a [Vec] of [UseItems] as input, ensures no
/// conflicting duplicates, groups them, and then emits as [Vec] (or multiple [Vec]) of [syn::ItemUse]
#[derive(Clone, Default, Debug)]
pub struct UseBuilder {
    map: IndexMap<UseKey, UseValue>,
    entries: usize,
}

impl UseBuilder {
    /// Create a new builder from a [Vec] of [UseItems]
    pub fn from_uses(items: Vec<UseItems>) -> Self {
        let mut root_map = Self {
            map: IndexMap::new(),
            entries: 0,
        };

        for inner_items in items {
            for item in inner_items.items {
                let data = UseData::new(item.vis, item.attrs, item.leading_colon.is_some());
                root_map.parse_tree(item.tree, data);
            }
        }

        root_map
    }

    fn add_node(&mut self, entry: UseKey, data: UseData) {
        match self.map.entry(entry) {
            indexmap::map::Entry::Occupied(mut e) => {
                e.get_mut().nodes.insert(data);
            }
            indexmap::map::Entry::Vacant(e) => {
                self.entries += 1;
                let mut u = UseValue::default();
                u.nodes.insert(data);
                e.insert(u);
            }
        }
    }

    fn add_path(&mut self, entry: UseKey) -> &mut UseBuilder {
        match self.map.entry(entry) {
            indexmap::map::Entry::Occupied(e) => &mut e.into_mut().paths,
            indexmap::map::Entry::Vacant(e) => {
                let u = UseValue::default();
                &mut e.insert(u).paths
            }
        }
    }

    fn parse_tree(&mut self, tree: syn::UseTree, data: UseData) {
        use syn::UseTree::*;

        match tree {
            Path(syn::UsePath { ident, tree, .. }) => {
                let map = self.add_path(UseKey::Name(ident));
                // TODO: I hate cloning tree here, but Box::into_inner() is unstable - replace when stable
                map.parse_tree(syn::UseTree::clone(&*tree), data);
            }
            Name(syn::UseName { ident }) => {
                self.add_node(UseKey::Name(ident), data);
            }
            Rename(syn::UseRename { ident, rename, .. }) => {
                self.add_node(UseKey::Rename(ident, rename), data);
            }
            Glob(syn::UseGlob { .. }) => {
                self.add_node(UseKey::Glob, data);
            }
            Group(syn::UseGroup { items, .. }) => {
                for item in items {
                    self.parse_tree(item, data.clone());
                }
            }
        }
    }

    fn next_map(
        use_map: UseBuilder,
        builder: ItemUseBuilder,
        items: &mut Vec<syn::ItemUse>,
    ) -> Result<(), Error> {
        let mut map: IndexMap<UseData, Vec<UseKey>> = IndexMap::new();
        let len = use_map.map.len();

        // Node Strategy: try to combine as we loop over
        for (key, value) in use_map.map {
            // *** Path handling **

            // Ignore anything but names for future paths (others are invalid as paths)
            if let UseKey::Name(path) = key.clone() {
                // Create a builder from the original
                let mut builder = builder.clone();
                builder.add_path(path);
                if let err @ Err(_) = Self::next_map(value.paths, builder, items) {
                    return err;
                }
            }

            // *** Node handling ***

            // Peek at nodes held by this key
            if !value.nodes.is_empty() {
                // We should really only have one entry - more than that means incompatible attrs
                if value.nodes.len() > 1 {
                    return Err(Error::UseWithDiffAttr);
                }

                // Insert into our map
                // Panic safety: we confirmed above there is exactly one entry
                match map.entry(value.nodes.into_iter().next().unwrap()) {
                    indexmap::map::Entry::Occupied(mut e) => {
                        e.get_mut().push(key);
                    }
                    indexmap::map::Entry::Vacant(e) => {
                        let mut set = Vec::with_capacity(len);
                        set.push(key);
                        e.insert(set);
                    }
                }
            }
        }

        // If we found any nodes, build them based on associated data
        for (data, names) in map {
            let item = builder.clone().into_item_use(names, data);
            items.push(item);
        }

        Ok(())
    }

    /// Consume this builder an emit a [Vec] of [syn::ItemUse]
    pub fn into_items(self) -> Result<Vec<syn::ItemUse>, Error> {
        let mut items = Vec::with_capacity(self.entries);
        let builder = ItemUseBuilder::default();
        Self::next_map(self, builder, &mut items)?;
        Ok(items)
    }

    /// Consume this builder and emit three vectors of [syn::ItemUse] partitioned by crate type:
    /// std, external, and intra-crate uses
    pub fn into_items_sections(self) -> Result<StdExtCrateUse, Error> {
        let items = self.into_items()?;

        // Will be too big - better too big than too small
        let mut std_uses = Vec::with_capacity(items.len());
        let mut extern_uses = Vec::with_capacity(items.len());
        let mut crate_uses = Vec::with_capacity(items.len());

        for item in items {
            use syn::UseTree::*;

            match &item.tree {
                // Name and rename don't make much sense, but technically legal
                Path(syn::UsePath { ident, .. })
                | Name(syn::UseName { ident })
                | Rename(syn::UseRename { ident, .. }) => {
                    let name = &*ident.to_string();

                    if STD.contains(&name) {
                        std_uses.push(item);
                    } else if CRATE.contains(&name) {
                        crate_uses.push(item);
                    } else {
                        extern_uses.push(item);
                    };
                }
                Glob(_) => return Err(Error::TopLevelGlob),
                Group(_) => {}
            }
        }

        Ok((std_uses, extern_uses, crate_uses))
    }
}

#[cfg(test)]
mod tests {
    use assert_unordered::assert_eq_unordered;
    use quote::quote;

    use crate::{UseBuilder, UseItems};

    fn make_builder() -> UseBuilder {
        let use1 = quote! {
            use crate::Test;
            use std::error::Error as StdError;
            use std::fmt::Debug;
        };

        let use2 = quote! {
            use syn::ItemUse;
            use std::fmt::Display;
            use crate::*;
        };

        let items1: UseItems = syn::parse2(use1).unwrap();
        let items2: UseItems = syn::parse2(use2).unwrap();

        UseBuilder::from_uses(vec![items1, items2])
    }

    #[test]
    fn items() {
        let builder = make_builder();
        //eprintln!("{:#?}", &builder);
        let uses = builder.into_items().unwrap();
        //println!("{uses:#?}");

        let expected = quote! {
            use crate::*;
            use std::error::Error as StdError;
            use std::fmt::{Debug, Display};
            use syn::ItemUse;
        };
        let expected = syn::parse2::<UseItems>(expected).unwrap().into_inner();

        assert_eq_unordered!(expected, uses);
    }

    #[test]
    fn items_separated() {
        let builder = make_builder();
        let (std_use, ext_use, crate_use) = builder.into_items_sections().unwrap();

        let std_expected = quote! {
            use std::error::Error as StdError;
            use std::fmt::{Debug, Display};
        };
        let std_expected = syn::parse2::<UseItems>(std_expected).unwrap().into_inner();

        let ext_expected = quote! {
            use syn::ItemUse;
        };
        let ext_expected = syn::parse2::<UseItems>(ext_expected).unwrap().into_inner();

        let crate_expected = quote! {
            use crate::*;
        };
        let crate_expected = syn::parse2::<UseItems>(crate_expected)
            .unwrap()
            .into_inner();

        assert_eq_unordered!(std_expected, std_use);
        assert_eq_unordered!(ext_expected, ext_use);
        assert_eq_unordered!(crate_expected, crate_use);
    }
}
