use std::collections::HashMap;

use flexstr::LocalStr;
use proc_macro2::TokenStream;


trait CodeFragment {
    fn generate(vars: HashMap<LocalStr, LocalStr>) -> TokenStream;
}
