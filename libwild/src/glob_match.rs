use globset::GlobBuilder;
use globset::GlobMatcher;
use regex_syntax::hir::HirKind;
use regex_syntax::hir::Look;
use std::str;
pub(crate) enum GlobPatternType {
    Exact,
    EscapedExact,
    Star,
    NonStar,
}

pub(crate) fn analyze_glob_pattern(pattern: &[u8]) -> GlobPatternType {
    // Fast path for when none of the characters are present.
    if memchr::memchr3(b'*', b'?', b'\\', pattern).is_none()
        && memchr::memchr2(b'[', b']', pattern).is_none()
    {
        return GlobPatternType::Exact;
    }

    let mut pattern_type = GlobPatternType::Exact;
    let mut it = pattern.iter();

    while let Some(&c) = it.next() {
        match c {
            b'\\' => {
                // Found an escape sequence, mark as EscapedExact if no globs found yet
                if matches!(pattern_type, GlobPatternType::Exact) {
                    pattern_type = GlobPatternType::EscapedExact;
                }
                it.next();
            }
            b'*' => {
                return GlobPatternType::Star;
            }
            b'[' | b']' | b'?' => {
                pattern_type = GlobPatternType::NonStar;
            }
            _ => {}
        }
    }

    pattern_type
}

#[derive(Clone, Debug)]
pub(crate) struct Pattern(GlobMatcher);

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.0.glob() == other.0.glob()
    }
}

impl Eq for Pattern {}

impl Pattern {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        self.0.glob().glob()
    }

    pub(crate) fn matches(&self, name: &str) -> bool {
        self.0.is_match(name)
    }
}

fn parse_glob(token: &[u8], escapes: bool) -> Result<globset::Glob, &'static str> {
    GlobBuilder::new(str::from_utf8(token).map_err(|_| "Invalid UTF-8 string")?)
        .backslash_escape(escapes)
        .build()
        .map_err(|_| "Invalid Glob Pattern")
}

pub(crate) fn unescape_pattern(pattern: &[u8]) -> Result<Vec<u8>, &'static str> {
    let glob = parse_glob(pattern, true)?;
    let hir = regex_syntax::ParserBuilder::new()
        .utf8(false)
        .build()
        .parse(glob.regex())
        .map_err(|_| "Invalid glob pattern")?;
    let HirKind::Concat(parts) = hir.kind() else {
        return Err("Expected literal glob");
    };
    let mut bytes = Vec::new();
    for part in parts {
        match part.kind() {
            HirKind::Look(Look::Start | Look::End) => (),
            HirKind::Literal(literal) => bytes.extend_from_slice(&literal.0),
            _ => return Err("Expected literal glob"),
        }
    }
    Ok(bytes)
}

pub(crate) fn compile_glob_pattern(token: &[u8]) -> Result<Pattern, &'static str> {
    Ok(Pattern(parse_glob(token, true)?.compile_matcher()))
}

pub(crate) fn compile_macho_glob_pattern(token: &[u8]) -> Result<Pattern, &'static str> {
    Ok(Pattern(parse_glob(token, false)?.compile_matcher()))
}

#[cfg(test)]
mod tests {
    use super::compile_macho_glob_pattern;

    #[test]
    fn macho_repeated_stars() {
        let pattern = compile_macho_glob_pattern(b"***_prefix***suffix**").unwrap();
        assert!(pattern.matches("_prefixsuffix"));
        assert!(pattern.matches("leading_prefix_middle_suffix_trailing"));
        assert!(!pattern.matches("_prefix_other"));
        assert!(!pattern.matches("_other_suffix"));
    }

    #[test]
    fn macho_stars_in_character_classes() {
        let pattern = compile_macho_glob_pattern(b"_class_[**]**").unwrap();
        assert!(pattern.matches("_class_*"));
        assert!(pattern.matches("_class_*_tail"));
        assert!(!pattern.matches("_class_x_tail"));
        assert!(!pattern.matches("_class_"));
    }

    #[test]
    fn macho_leading_close_bracket_in_character_classes() {
        let pattern = compile_macho_glob_pattern(b"_class_[]*]**").unwrap();
        assert!(pattern.matches("_class_]_tail"));
        assert!(pattern.matches("_class_*_tail"));
        assert!(!pattern.matches("_class_x_tail"));

        let negated = compile_macho_glob_pattern(b"_class_[!]*]**").unwrap();
        assert!(negated.matches("_class_x_tail"));
        assert!(!negated.matches("_class_]_tail"));
        assert!(!negated.matches("_class_*_tail"));
    }

    #[test]
    fn macho_backslashes_are_literal() {
        let pattern = compile_macho_glob_pattern(br"_prefix\**").unwrap();
        assert!(pattern.matches(r"_prefix\"));
        assert!(pattern.matches(r"_prefix\suffix"));
        assert!(!pattern.matches("_prefixsuffix"));
        assert!(!pattern.matches("_prefix*"));
    }
}
