//! Contains parse functions that may be convenient for implementing language plugins.

use nom::{branch, bytes, character, combinator, error::VerboseError, multi, sequence, IResult};

/// Parses a string delimited by double quotes. Trims.
pub fn string(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
    combinator::map(
        sequence::delimited(
            character::complete::char('"'),
            bytes::complete::is_not("\""),
            character::complete::char('"'),
        ),
        str::trim,
    )(i)
}

/// Parses a string delimited by single quotes. Trims.
pub fn string_single(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
    combinator::map(
        sequence::delimited(
            character::complete::char('\''),
            bytes::complete::is_not("'"),
            character::complete::char('\''),
        ),
        str::trim,
    )(i)
}

/// Parses a comma-separated list of double quote strings like "a", "b", "c".
pub fn comma_separated_strings(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(string, i)
}

/// Parses a comma-separated list of single quote strings like 'a', 'b', 'c'.
pub fn comma_separated_strings_single(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(string_single, i)
}

/// Parses a comma-separated list of mixed quote strings like 'a', "b", 'c'.
pub fn comma_separated_strings_either(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(branch::alt((string, string_single)), i)
}

/// Parses a comma-separated list of things, thing being defined by the parser given to the function.
fn comma_separated_things<'a>(
    thing_parser: impl FnMut(&'a str) -> IResult<&'a str, &'a str, VerboseError<&'a str>>,
    i: &'a str,
) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    multi::separated_list1(
        sequence::delimited(
            character::complete::multispace0,
            character::complete::char(','),
            character::complete::multispace0,
        ),
        thing_parser,
    )(i)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn parses_string() {
        let (left, res) = string("\"abcd\"").unwrap();
        assert!(left.is_empty());
        assert_eq!(res, "abcd");
    }

    #[test]
    fn parses_string_single() {
        let (left, res) = string_single("'abcd'").unwrap();
        assert!(left.is_empty());
        assert_eq!(res, "abcd");
    }

    #[test]
    fn parses_comma_separated_strings() {
        let (left, res) = comma_separated_strings("\"abcd\", \"efgh\", \"hijk\"").unwrap();
        assert!(left.is_empty());
        assert_eq!(res, &["abcd", "efgh", "hijk"]);
    }

    #[test]
    fn parses_comma_separated_strings_single() {
        let (left, res) = comma_separated_strings_single("'abcd', 'efgh', 'hijk'").unwrap();
        assert!(left.is_empty());
        assert_eq!(res, &["abcd", "efgh", "hijk"]);
    }

    #[test]
    fn parses_comma_separated_strings_either() {
        let (left, res) = comma_separated_strings_either("'abcd', \"efgh\", 'hijk'").unwrap();
        assert!(left.is_empty());
        assert_eq!(res, &["abcd", "efgh", "hijk"]);
    }
}
