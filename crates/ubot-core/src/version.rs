//! Versions and version constraints, a port of uBotCore's `UBotSemVer.h` (SPEC §2).
//!
//! Version parsing is lenient about the release part ("1", "v1.2"); build metadata is ignored.
//! A constraint is empty or "*" (any version), or whitespace separated comparators that must all
//! hold: `[op]version` with op in `= >= > <= < ^ ~` (no op means `=`), an operator may be separated
//! from its version by whitespace. Partial versions are X-ranges, and every upper bound produced by
//! `^`, `~` or a partial version excludes the prereleases of that bound.

use std::cmp::Ordering;
use std::fmt;

const MAX_COMPONENT: u32 = i32::MAX as u32;

#[derive(Clone, Debug, Default)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// Dot separated identifiers after '-', without the dash. Empty for a release.
    pub pre: String,
}

impl SemVer {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        SemVer {
            major,
            minor,
            patch,
            pre: String::new(),
        }
    }

    pub fn with_pre(major: u32, minor: u32, patch: u32, pre: &str) -> Self {
        SemVer {
            major,
            minor,
            patch,
            pre: pre.to_string(),
        }
    }

    /// Accepts "1", "1.2", "1.2.3", an optional leading 'v'/'V', an optional "-prerelease" and
    /// ignores "+build". Surrounding whitespace is ignored.
    pub fn parse(text: &str) -> Option<SemVer> {
        parse_version(text).map(|(version, _)| version)
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    pub fn compare(&self, other: &SemVer) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then_with(|| compare_prerelease(&self.pre, &other.pre))
    }
}

impl PartialEq for SemVer {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl Eq for SemVer {}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn is_numeric_identifier(identifier: &str) -> bool {
    !identifier.is_empty() && identifier.bytes().all(|b| b.is_ascii_digit())
}

fn parse_component(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    let mut value: u64 = 0;
    for b in text.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u64::from(b - b'0');
        if value > u64::from(MAX_COMPONENT) {
            return None;
        }
    }
    Some(value as u32)
}

fn is_valid_prerelease(pre: &str) -> bool {
    !pre.is_empty()
        && pre
            .split('.')
            .all(|identifier| !identifier.is_empty() && identifier.chars().all(is_identifier_char))
}

/// Shared by [`SemVer::parse`] and the constraint parser, which needs the number of release
/// components that were written to expand partial versions.
fn parse_version(input: &str) -> Option<(SemVer, usize)> {
    let mut text = input.trim();
    if text.starts_with(['v', 'V']) {
        text = &text[1..];
    }
    if let Some(plus) = text.find('+') {
        text = &text[..plus];
    }

    let (release, pre) = match text.find('-') {
        Some(dash) => {
            let pre = &text[dash + 1..];
            if !is_valid_prerelease(pre) {
                return None;
            }
            (&text[..dash], pre)
        }
        None => (text, ""),
    };

    // An empty release has no components at all (UE's ParseIntoArray yields nothing for "").
    if release.is_empty() {
        return None;
    }
    let components: Vec<&str> = release.split('.').collect();
    if components.len() > 3 {
        return None;
    }
    let mut values = [0u32; 3];
    for (index, component) in components.iter().enumerate() {
        values[index] = parse_component(component)?;
    }
    Some((
        SemVer::with_pre(values[0], values[1], values[2], pre),
        components.len(),
    ))
}

fn compare_numeric_identifiers(a: &str, b: &str) -> Ordering {
    // Compared as digit strings so arbitrarily long identifiers cannot overflow.
    let a = trim_leading_zeros(a);
    let b = trim_leading_zeros(b);
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn trim_leading_zeros(digits: &str) -> &str {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() && !digits.is_empty() {
        &digits[digits.len() - 1..]
    } else {
        trimmed
    }
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        // A release has higher precedence than any of its prereleases.
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    let ids_a: Vec<&str> = a.split('.').collect();
    let ids_b: Vec<&str> = b.split('.').collect();
    for (id_a, id_b) in ids_a.iter().zip(ids_b.iter()) {
        let numeric_a = is_numeric_identifier(id_a);
        let numeric_b = is_numeric_identifier(id_b);
        let order = match (numeric_a, numeric_b) {
            (true, true) => compare_numeric_identifiers(id_a, id_b),
            // Numeric identifiers always have lower precedence than alphanumeric ones.
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => id_a.cmp(id_b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    ids_a.len().cmp(&ids_b.len())
}

/// Smallest version above every version sharing `version`'s components up to `component`
/// (0 = major, 1 = minor, 2 = patch), as `X.Y.Z-0` so the bound's prereleases are excluded too.
/// `None` when the component is already at its maximum.
fn make_upper_bound(version: &SemVer, component: usize) -> Option<SemVer> {
    let mut values = [version.major, version.minor, version.patch];
    if values[component] == MAX_COMPONENT {
        return None;
    }
    values[component] += 1;
    for value in values.iter_mut().skip(component + 1) {
        *value = 0;
    }
    Some(SemVer::with_pre(values[0], values[1], values[2], "0"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Equal,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Comparator {
    comparison: Comparison,
    version: SemVer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constraint {
    comparators: Vec<Comparator>,
    text: String,
}

// Two character operators first so ">=" is not read as ">" followed by "=1.0".
const OPERATORS: [&str; 7] = [">=", "<=", ">", "<", "=", "^", "~"];

impl Constraint {
    /// The constraint that every version satisfies.
    pub fn any() -> Self {
        Constraint::default()
    }

    pub fn parse(input: &str) -> Result<Constraint, String> {
        let tokens: Vec<&str> = input.split_whitespace().collect();
        let mut comparators = Vec::new();
        let mut normalized = Vec::new();

        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index];
            index += 1;
            if token == "*" {
                normalized.push(token.to_string());
                continue;
            }

            let operator = OPERATORS
                .iter()
                .copied()
                .find(|op| token.starts_with(op))
                .unwrap_or("");
            let mut version_text = &token[operator.len()..];
            if version_text.is_empty() {
                if index >= tokens.len() {
                    return Err(format!(
                        "Operator '{operator}' has no version in constraint '{input}'"
                    ));
                }
                version_text = tokens[index];
                index += 1;
            }

            let Some((version, count)) = parse_version(version_text) else {
                return Err(format!(
                    "Invalid version '{version_text}' in constraint '{input}'"
                ));
            };

            // A prerelease pins an exact version, so "1.2-beta" is treated like "1.2.0-beta".
            let effective_count = if version.is_prerelease() { 3 } else { count };
            let partial = effective_count < 3;
            let last_component = effective_count - 1;

            let mut add = |comparison: Comparison, bound: SemVer| {
                comparators.push(Comparator {
                    comparison,
                    version: bound,
                })
            };
            match operator {
                "^" => {
                    // Bump the left-most non-zero component; when every written component is zero,
                    // bump the last written one (^0 -> <1.0.0, ^0.0 -> <0.1.0, ^0.0.0 -> <0.0.1).
                    let bump = if version.major != 0 || effective_count == 1 {
                        0
                    } else if version.minor != 0 || effective_count == 2 {
                        1
                    } else {
                        2
                    };
                    add(Comparison::GreaterOrEqual, version.clone());
                    if let Some(bound) = make_upper_bound(&version, bump) {
                        add(Comparison::Less, bound);
                    }
                }
                "~" => {
                    add(Comparison::GreaterOrEqual, version.clone());
                    if let Some(bound) =
                        make_upper_bound(&version, if effective_count == 1 { 0 } else { 1 })
                    {
                        add(Comparison::Less, bound);
                    }
                }
                ">=" => add(Comparison::GreaterOrEqual, version.clone()),
                ">" => {
                    if partial {
                        // Greater than every release of the X-range: at least the next release.
                        match make_upper_bound(&version, last_component) {
                            Some(mut bound) => {
                                bound.pre.clear();
                                add(Comparison::GreaterOrEqual, bound);
                            }
                            None => add(
                                Comparison::Greater,
                                SemVer::new(MAX_COMPONENT, MAX_COMPONENT, MAX_COMPONENT),
                            ),
                        }
                    } else {
                        add(Comparison::Greater, version.clone());
                    }
                }
                "<" => {
                    if partial {
                        add(
                            Comparison::Less,
                            SemVer::with_pre(version.major, version.minor, version.patch, "0"),
                        );
                    } else {
                        add(Comparison::Less, version.clone());
                    }
                }
                "<=" => {
                    if partial {
                        if let Some(bound) = make_upper_bound(&version, last_component) {
                            add(Comparison::Less, bound);
                        }
                    } else {
                        add(Comparison::LessOrEqual, version.clone());
                    }
                }
                _ => {
                    // "=" or no operator.
                    if partial {
                        add(Comparison::GreaterOrEqual, version.clone());
                        if let Some(bound) = make_upper_bound(&version, last_component) {
                            add(Comparison::Less, bound);
                        }
                    } else {
                        add(Comparison::Equal, version.clone());
                    }
                }
            }

            normalized.push(format!("{operator}{version_text}"));
        }

        Ok(Constraint {
            comparators,
            text: normalized.join(" "),
        })
    }

    pub fn is_satisfied_by(&self, version: &SemVer) -> bool {
        self.comparators.iter().all(|comparator| {
            let order = version.compare(&comparator.version);
            match comparator.comparison {
                Comparison::Equal => order == Ordering::Equal,
                Comparison::Greater => order == Ordering::Greater,
                Comparison::GreaterOrEqual => order != Ordering::Less,
                Comparison::Less => order == Ordering::Less,
                Comparison::LessOrEqual => order != Ordering::Greater,
            }
        })
    }

    /// True when the constraint has no comparators ("" or "*").
    pub fn is_any(&self) -> bool {
        self.comparators.is_empty()
    }

    /// The parsed text with whitespace normalized.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// True when `version` parses and satisfies `constraint` (which must parse too).
pub fn satisfies(version: &str, constraint: &str) -> bool {
    match (SemVer::parse(version), Constraint::parse(constraint)) {
        (Some(version), Ok(constraint)) => constraint.is_satisfied_by(&version),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_cases(cases: &[(&str, &str, bool)]) {
        for (constraint, version, expected) in cases {
            let parsed = Constraint::parse(constraint)
                .unwrap_or_else(|e| panic!("'{constraint}' should parse: {e}"));
            let version_value =
                SemVer::parse(version).unwrap_or_else(|| panic!("'{version}' should parse"));
            assert_eq!(
                parsed.is_satisfied_by(&version_value),
                *expected,
                "'{constraint}' {} be satisfied by {version}",
                if *expected { "should" } else { "should not" }
            );
        }
    }

    #[test]
    fn parses_valid_versions() {
        let cases: &[(&str, u32, u32, u32, &str, &str)] = &[
            ("1", 1, 0, 0, "", "1.0.0"),
            ("1.2", 1, 2, 0, "", "1.2.0"),
            ("1.2.3", 1, 2, 3, "", "1.2.3"),
            ("v1.2.3", 1, 2, 3, "", "1.2.3"),
            ("V0.1", 0, 1, 0, "", "0.1.0"),
            ("  2.0.1  ", 2, 0, 1, "", "2.0.1"),
            ("1.2.3-beta.1", 1, 2, 3, "beta.1", "1.2.3-beta.1"),
            ("1.2.3+build.7", 1, 2, 3, "", "1.2.3"),
            ("1.2.3-rc.1+exp.sha.5114f85", 1, 2, 3, "rc.1", "1.2.3-rc.1"),
            ("1.0.0-x-y-z.0", 1, 0, 0, "x-y-z.0", "1.0.0-x-y-z.0"),
            ("1.2-alpha", 1, 2, 0, "alpha", "1.2.0-alpha"),
            ("2147483647.0.0", 2147483647, 0, 0, "", "2147483647.0.0"),
        ];
        for (text, major, minor, patch, pre, canonical) in cases {
            let version = SemVer::parse(text).unwrap_or_else(|| panic!("'{text}' parses"));
            assert_eq!(
                (version.major, version.minor, version.patch),
                (*major, *minor, *patch),
                "{text}"
            );
            assert_eq!(version.pre, *pre, "{text}");
            assert_eq!(version.to_string(), *canonical, "{text}");
            assert_eq!(version.is_prerelease(), !pre.is_empty(), "{text}");
        }
    }

    #[test]
    fn rejects_invalid_versions() {
        for text in [
            "",
            "   ",
            "v",
            "1.2.3.4",
            "1..2",
            "1.",
            ".1",
            "a",
            "1.a",
            "1.2.x",
            "1.2.3-",
            "1.2.3-beta..1",
            "1.2.3-beta_1",
            "-1",
            "+build",
            "1 .2",
            "2147483648",
            "1.-2.3",
        ] {
            assert!(SemVer::parse(text).is_none(), "'{text}' is rejected");
        }
    }

    #[test]
    fn orders_by_precedence() {
        let ascending = [
            "0.9.9",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
            "10.0.0",
        ];
        let versions: Vec<SemVer> = ascending
            .iter()
            .map(|text| SemVer::parse(text).unwrap())
            .collect();
        for (index, version) in versions.iter().enumerate() {
            assert_eq!(version.compare(version), Ordering::Equal);
            for later in &versions[index + 1..] {
                assert_eq!(
                    version.compare(later),
                    Ordering::Less,
                    "{version} < {later}"
                );
                assert_eq!(
                    later.compare(version),
                    Ordering::Greater,
                    "{later} > {version}"
                );
            }
        }

        let v = |text: &str| SemVer::parse(text).unwrap();
        assert_eq!(
            v("1.0"),
            v("1.0.0+build.5"),
            "partial and build metadata compare equal"
        );
        assert_eq!(
            v("1.0.0-beta.01"),
            v("1.0.0-beta.1"),
            "numeric identifiers compare by value"
        );
        assert!(
            v("1.0.0-Beta") < v("1.0.0-beta"),
            "alphanumeric identifiers compare in ASCII order"
        );
        assert!(
            v("1.0.0-beta.99999999999999999999") < v("1.0.0-beta.100000000000000000000"),
            "no overflow"
        );

        let low = SemVer::new(1, 2, 3);
        let high = SemVer::new(1, 3, 0);
        assert!(low < high && low <= high && low <= low);
        assert!(high > low && high >= low && high >= high);
        assert!(low != high);
        assert_eq!(low, SemVer::new(1, 2, 3));
    }

    #[test]
    fn constraint_syntax() {
        let empty = Constraint::parse("").unwrap();
        assert!(empty.is_any());
        assert!(empty.is_satisfied_by(&SemVer::with_pre(0, 0, 1, "alpha")));
        assert!(Constraint::parse("   ").unwrap().is_any());

        let star = Constraint::parse("*").unwrap();
        assert!(star.is_any());
        assert_eq!(star.as_str(), "*");

        let spaced = Constraint::parse("  >=1.0    <2.0 ").unwrap();
        assert!(!spaced.is_any());
        assert_eq!(spaced.as_str(), ">=1.0 <2.0");
        assert_eq!(
            Constraint::parse(">= 1.0 < 2.0").unwrap().to_string(),
            ">=1.0 <2.0"
        );

        check_cases(&[
            (">= 1.0 < 2.0", "1.5.0", true),
            (">= 1.0 < 2.0", "2.0.0", false),
            ("* >=1.0", "0.9.0", false),
            ("* >=1.0", "1.0.0", true),
            ("\t^v1.2\n", "1.9.0", true),
        ]);

        for text in [
            ">=",
            "^",
            "~ ",
            ">=1.0 <",
            ">=abc",
            "1.2.3.4",
            "!=1.0",
            "=>1.0",
            "==1.0",
            "1.x",
            ">=*",
            ">=1.0,<2.0",
            "latest",
        ] {
            let error = Constraint::parse(text).expect_err(text);
            assert!(!error.is_empty(), "'{text}' reports an error");
        }
    }

    #[test]
    fn caret_and_tilde() {
        check_cases(&[
            ("^1.2.3", "1.2.3", true),
            ("^1.2.3", "1.9.9", true),
            ("^1.2.3", "1.3.0-beta", true),
            ("^1.2.3", "1.2.2", false),
            ("^1.2.3", "1.2.3-rc.1", false),
            ("^1.2.3", "2.0.0", false),
            ("^1.2.3", "2.0.0-beta", false),
            ("^0.2.3", "0.2.3", true),
            ("^0.2.3", "0.2.9", true),
            ("^0.2.3", "0.2.2", false),
            ("^0.2.3", "0.3.0", false),
            ("^0.2.3", "1.0.0", false),
            ("^0.0.3", "0.0.3", true),
            ("^0.0.3", "0.0.4", false),
            ("^0.0.3", "0.0.2", false),
            ("^0.1.0", "0.1.0", true),
            ("^0.1.0", "0.1.7", true),
            ("^0.1.0", "0.2.0", false),
            ("^0.1.0", "0.2.0-dev", false),
            ("^1", "1.0.0", true),
            ("^1", "1.99.0", true),
            ("^1", "2.0.0", false),
            ("^1.2", "1.2.0", true),
            ("^1.2", "1.9.0", true),
            ("^1.2", "1.1.9", false),
            ("^0", "0.9.9", true),
            ("^0", "1.0.0", false),
            ("^0.0", "0.0.9", true),
            ("^0.0", "0.1.0", false),
            ("^0.0.0", "0.0.0", true),
            ("^0.0.0", "0.0.1", false),
            ("^0.2", "0.2.5", true),
            ("^0.2", "0.3.0", false),
            ("^1.0.0-rc.1", "1.0.0-rc.2", true),
            ("^1.0.0-rc.1", "1.0.0-beta", false),
            ("^1.0.0-rc.1", "1.0.0", true),
            ("^1.0.0-rc.1", "2.0.0-rc.1", false),
            ("~1.2.3", "1.2.3", true),
            ("~1.2.3", "1.2.9", true),
            ("~1.2.3", "1.2.2", false),
            ("~1.2.3", "1.3.0", false),
            ("~1.2.3", "1.3.0-alpha", false),
            ("~1.2", "1.2.0", true),
            ("~1.2", "1.2.9", true),
            ("~1.2", "1.1.9", false),
            ("~1.2", "1.3.0", false),
            ("~1", "1.0.0", true),
            ("~1", "1.9.0", true),
            ("~1", "0.9.9", false),
            ("~1", "2.0.0", false),
            ("~0.0.3", "0.0.9", true),
            ("~0.0.3", "0.1.0", false),
            ("~5.5", "5.5.4", true),
            ("~5.5", "5.6.0", false),
        ]);
    }

    #[test]
    fn comparators() {
        check_cases(&[
            ("=1.2.3", "1.2.3", true),
            ("=1.2.3", "1.2.4", false),
            ("=1.2.3", "1.2.3-beta", false),
            ("1.2.3", "1.2.3", true),
            ("1.2.3", "1.2.2", false),
            ("=1.2.3-beta", "1.2.3-beta", true),
            ("=1.2.3-beta", "1.2.3", false),
            (">=1.2.3", "1.2.3", true),
            (">=1.2.3", "1.2.2", false),
            (">=1.2.3", "1.2.3-rc.1", false),
            (">1.2.3", "1.2.3", false),
            (">1.2.3", "1.2.4", true),
            (">1.2.3", "1.2.4-alpha", true),
            ("<=1.2.3", "1.2.3", true),
            ("<=1.2.3", "1.2.4", false),
            ("<1.2.3", "1.2.3", false),
            ("<1.2.3", "1.2.2", true),
            ("<1.2.3", "1.2.3-beta", true),
            ("1.2", "1.2.0", true),
            ("1.2", "1.2.7", true),
            ("1.2", "1.1.9", false),
            ("1.2", "1.3.0", false),
            ("1.2", "1.3.0-alpha", false),
            ("=1", "1.5.0", true),
            ("=1", "2.0.0", false),
            (">1.2", "1.2.9", false),
            (">1.2", "1.3.0", true),
            (">1", "1.9.9", false),
            (">1", "2.0.0", true),
            (">=1.2", "1.2.0", true),
            (">=1.2", "1.1.9", false),
            (">=1.0", "1.0.0", true),
            (">=1.0", "7.3.1", true),
            ("<1.2", "1.1.9", true),
            ("<1.2", "1.2.0", false),
            ("<1.2", "1.2.0-beta", false),
            ("<=1.2", "1.2.9", true),
            ("<=1.2", "1.3.0", false),
            ("<=1", "1.9.9", true),
            ("<=1", "2.0.0", false),
            (">=1.2.0 <1.5.0", "1.2.0", true),
            (">=1.2.0 <1.5.0", "1.4.9", true),
            (">=1.2.0 <1.5.0", "1.5.0", false),
            (">=1.2.0 <1.5.0", "1.1.0", false),
            ("^1.2 <1.4", "1.3.9", true),
            ("^1.2 <1.4", "1.4.0", false),
            (">1.0.0 <=1.0.5 1.0", "1.0.5", true),
            (">1.0.0 <=1.0.5 1.0", "1.0.0", false),
            (">=2.0 <1.0", "1.5.0", false),
            (">=1.0.0-beta.2", "1.0.0-beta.11", true),
            (">=1.0.0-beta.2", "1.0.0-beta.1", false),
            (">=1.0.0-beta.2", "1.0.0-alpha", false),
            (">=1.0.0-beta.2", "1.0.0", true),
            (">1.0.0-alpha <1.0.0", "1.0.0-alpha.1", true),
            (">1.0.0-alpha <1.0.0", "1.0.0-rc.1", true),
            (">1.0.0-alpha <1.0.0", "1.0.0", false),
        ]);
    }

    #[test]
    fn maximum_components_have_no_upper_bound() {
        check_cases(&[
            ("^2147483647", "2147483647.5.0", true),
            (">2147483647", "2147483647.9.9", false),
            ("<=2147483647", "2147483647.9.9", true),
        ]);
        assert!(satisfies("0.1.0", "^0.1.0"));
        assert!(!satisfies("bad", "^0.1.0"));
        assert!(!satisfies("0.1.0", "bad"));
    }
}
