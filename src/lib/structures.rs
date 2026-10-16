use compact_str::CompactString;
use base64::Engine;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use std::fmt;

pub type Map<K, V> = HashMap<K, V, FxBuildHasher>;

#[derive(Default)]
pub struct Config {
    pub ints:   Map<CompactString, i64>,
    pub strs:   Map<CompactString, CompactString>,
    pub floats: Map<CompactString, f64>,
    pub bytes: Map<CompactString, Vec<u8>>,
    pub bools: Map<CompactString, bool>,
    pub inners: Map<CompactString, Config>,
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_indent(f, 0)
    }
}

impl Config {
    #[inline]
    fn write_indent(f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        for _ in 0..indent {
            f.write_str("  ")?;
        }
        Ok(())
    }

    fn fmt_with_indent(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        for (k, v) in &self.ints {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} = {v} (int)")?;
        }
        for (k, v) in &self.strs {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} = \"{v}\" (str)")?;
        }
        for (k, v) in &self.floats {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} = {v} (float)")?;
        }
        for (k, v) in &self.bytes {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} = {} (bytes)", base64::engine::general_purpose::STANDARD.encode(v))?;
        }
        for (k, v) in &self.bools {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} = {v} (bool)")?;
        }
        for (k, v) in &self.inners {
            Self::write_indent(f, indent)?;
            writeln!(f, "{k} {{")?;
            v.fmt_with_indent(f, indent + 1)?;
            Self::write_indent(f, indent)?;
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}