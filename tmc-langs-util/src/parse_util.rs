use nom::{branch, bytes, character, combinator, error::VerboseError, multi, sequence, IResult};

/// Parses "a", "b", "c". Takes care of whitespace before and after.
pub fn comma_separated_strings(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(
        sequence::delimited(
            character::complete::char('"'),
            bytes::complete::is_not("\""),
            character::complete::char('"'),
        ),
        i,
    )
}

/// Parses 'a', 'b', 'c'. Takes care of whitespace before and after.
pub fn comma_separated_strings_single(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(
        sequence::delimited(
            character::complete::char('\''),
            bytes::complete::is_not("'"),
            character::complete::char('\''),
        ),
        i,
    )
}

/// Parses 'a', "b", 'c'. Takes care of whitespace before and after.
pub fn comma_separated_strings_either(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    comma_separated_things(
        branch::alt((
            sequence::delimited(
                character::complete::char('"'),
                bytes::complete::is_not("\""),
                character::complete::char('"'),
            ),
            sequence::delimited(
                character::complete::char('\''),
                bytes::complete::is_not("'"),
                character::complete::char('\''),
            ),
        )),
        i,
    )
}

fn comma_separated_things<'a>(
    thing_parser: impl FnMut(&'a str) -> IResult<&'a str, &'a str, VerboseError<&'a str>>,
    i: &'a str,
) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
    sequence::delimited(
        character::complete::multispace0,
        multi::separated_list1(
            sequence::delimited(
                character::complete::multispace0,
                character::complete::char(','),
                character::complete::multispace0,
            ),
            thing_parser,
        ),
        character::complete::multispace0,
    )(i)
}
