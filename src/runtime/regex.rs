use crate::value::{AnyCollect, NativeClassBuilder};

use gc_arena::collect::Trace;
use regex::Regex;
use std::any::Any;

#[derive(Debug)]
pub struct NativeRegexState {
    pub regex: Regex,
}

impl NativeRegexState {
    pub fn new(regex: Regex) -> Self {
        Self { regex }
    }
}

impl AnyCollect for NativeRegexState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {
        // No GC references in Regex
    }
}

pub fn build_regex_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Regex", Some("Object"))
}
