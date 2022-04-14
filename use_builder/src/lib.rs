use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::{cmp, fmt, hash};

const STD: [&str; 5] = ["std", "alloc", "core", "proc_macro", "test"];
const CRATE: [&str; 3] = ["self", "super", "crate"];

// *** UseItems ***

pub struct UseItems {
    items: Vec<syn::ItemUse>,
}

impl syn::parse::Parse for UseItems {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Random guess on capacity
        let mut items = Vec::with_capacity(5);

        while !input.is_empty() {
            items.push(input.parse()?)
        }

        Ok(Self { items })
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

#[derive(Clone, cmp::Eq, hash::Hash, cmp::PartialEq)]
enum UseKey {
    Name(syn::Ident),
    Rename(syn::Ident, syn::Ident),
    Glob,
}

// *** Use Data ***

#[derive(Clone, cmp::Eq, hash::Hash, cmp::PartialEq)]
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

#[derive(Clone, Default)]
struct UseValue {
    nodes: HashSet<UseData>,
    paths: UseMap,
}

// *** Error ***

#[derive(fmt::Debug)]
pub enum Error {
    TopLevelGlob,
    TopLevelGroup,
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
    fn add_path(&mut self, path: syn::Ident) {
        self.paths.push(path);
    }

    fn into_item_use(mut self, mut names: Vec<UseKey>, data: UseData) -> syn::ItemUse {
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
            key_to_tree(names.remove(0))
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

pub type StdExtCrateUse = (Vec<syn::ItemUse>, Vec<syn::ItemUse>, Vec<syn::ItemUse>);

#[derive(Clone, Default)]
pub struct UseMap {
    map: HashMap<UseKey, UseValue>,
    entries: usize,
}

impl UseMap {
    pub fn from_uses(items: Vec<UseItems>) -> Self {
        let mut root_map = Self {
            map: HashMap::new(),
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
            Entry::Occupied(mut e) => {
                self.entries += 1;
                e.get_mut().nodes.insert(data);
            }
            Entry::Vacant(e) => {
                let mut u = UseValue::default();
                u.nodes.insert(data);
                e.insert(u);
            }
        }
    }

    fn add_path(&mut self, entry: UseKey) -> &mut UseMap {
        match self.map.entry(entry) {
            Entry::Occupied(e) => &mut e.into_mut().paths,
            Entry::Vacant(e) => {
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
        use_map: UseMap,
        mut builder: ItemUseBuilder,
        items: &mut Vec<syn::ItemUse>,
    ) -> Result<(), Error> {
        let mut map: HashMap<UseData, Vec<UseKey>> = HashMap::new();
        let len = use_map.map.len();

        // Node Strategy: try to combine as we loop over
        for (key, value) in use_map.map {
            // *** Path handling **

            // Ignore anything but names for future paths (others are invalid as paths)
            if let UseKey::Name(path) = key.clone() {
                builder.add_path(path);
                if let err @ Err(_) = Self::next_map(value.paths, builder.clone(), items) {
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
                    Entry::Occupied(mut e) => {
                        e.get_mut().push(key);
                    }
                    Entry::Vacant(e) => {
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

    pub fn optimize(self) -> Result<Vec<syn::ItemUse>, Error> {
        let mut items = Vec::with_capacity(self.entries);
        let builder = ItemUseBuilder::default();
        Self::next_map(self, builder, &mut items)?;
        Ok(items)
    }

    pub fn optimize_sections(self) -> Result<StdExtCrateUse, Error> {
        let items = self.optimize()?;

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
mod tests {}
