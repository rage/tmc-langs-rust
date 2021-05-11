//! Contains utilities for parsing annotated exercise source files, separating lines into
//! strings, stubs and solutions so that they can be more easily filtered later.

use crate::TmcError;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use std::io::{BufRead, BufReader, Read};

// rules for finding comments in various languages
static META_SYNTAXES_C: Lazy<[MetaSyntax; 2]> = Lazy::new(|| {
    [
        MetaSyntax::new("//", None),
        MetaSyntax::new(r"/\*", Some(r"\*/")),
    ]
});
static META_SYNTAXES_HTML: Lazy<[MetaSyntax; 1]> =
    Lazy::new(|| [MetaSyntax::new("<!--", Some("-->"))]);
static META_SYNTAXES_PY: Lazy<[MetaSyntax; 1]> = Lazy::new(|| [MetaSyntax::new("#", None)]);

/// Used to classify lines of code based on the annotations in the file.
#[derive(Debug, PartialEq, Eq)]
pub enum MetaString {
    String(String),
    Stub(String),
    Solution(String),
    Hidden(String),
    SolutionFileMarker,
    HiddenFileMarker,
}

/// Contains the needed regexes for a given comment syntax.
#[derive(Debug)]
struct MetaSyntax {
    solution_file: Regex,
    solution_begin: Regex,
    solution_end: Regex,
    stub_begin: Regex,
    stub_end: Regex,
    hidden_file: Regex,
    hidden_begin: Regex,
    hidden_end: Regex,
}

#[allow(clippy::unwrap_used)]
impl MetaSyntax {
    fn new(comment_start: &'static str, comment_end: Option<&'static str>) -> Self {
        // comment patterns
        let comment_start_pattern = format!(r"^(\s*){}\s*", comment_start);
        let comment_end_pattern = match comment_end {
            Some(s) => format!(r"(.*){}\s*", s),
            None => "(.*)".to_string(),
        };

        // annotation patterns
        let solution_file = Regex::new(&format!(
            r"{}SOLUTION\s+FILE{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let solution_begin = Regex::new(&format!(
            r"{}BEGIN\s+SOLUTION{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let solution_end = Regex::new(&format!(
            r"{}END\s+SOLUTION{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let stub_begin =
            Regex::new(&format!(r"{}STUB:[\s&&[^\n]]*", comment_start_pattern)).unwrap();
        let stub_end = Regex::new(&comment_end_pattern).unwrap();
        let hidden_file = Regex::new(&format!(
            r"{}HIDDEN\s+FILE{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let hidden_begin = Regex::new(&format!(
            r"{}BEGIN\s+HIDDEN{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let hidden_end = Regex::new(&format!(
            r"{}END\s+HIDDEN{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();

        Self {
            solution_file,
            solution_begin,
            solution_end,
            stub_begin,
            stub_end,
            hidden_file,
            hidden_begin,
            hidden_end,
        }
    }
}

/// Parses a given text file into an iterator of `MetaString`s.
#[derive(Debug)]
pub struct MetaSyntaxParser<B: BufRead> {
    meta_syntaxes: &'static [MetaSyntax],
    reader: B,
    // contains the syntax that started the current stub block
    // used to make sure only the appropriate terminator ends the block
    in_stub: Option<&'static MetaSyntax>,
    in_solution: bool,
    in_hidden: bool,
}

impl<R: Read> MetaSyntaxParser<BufReader<R>> {
    pub fn new(target: R, target_extension: &str) -> Self {
        let reader = BufReader::new(target);
        // assigns each supported file extension with the proper comment syntax
        let meta_syntaxes: &[MetaSyntax] = match target_extension {
            "java" | "c" | "cpp" | "h" | "hpp" | "js" | "css" | "rs" | "qml" => &*META_SYNTAXES_C,
            "xml" | "http" | "html" | "qrc" => &*META_SYNTAXES_HTML,
            "properties" | "py" | "R" | "pro" => &*META_SYNTAXES_PY,
            _ => &[],
        };

        Self {
            meta_syntaxes,
            reader,
            in_stub: None,
            in_solution: false,
            in_hidden: false,
        }
    }
}

// iterates through the lines in the underlying file, parsing them to MetaStrings
impl<B: BufRead> Iterator for MetaSyntaxParser<B> {
    type Item = Result<MetaString, TmcError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut raw_string_buffer: Vec<u8> = Vec::new();

        match self.reader.read_until(b'\n', &mut raw_string_buffer) {
            // read 0 bytes = reader empty = iterator empty
            Ok(0) => None,
            Ok(_) => {
                let mut s = String::from_utf8_lossy(&raw_string_buffer).to_string();
                // check line with each meta syntax
                for meta_syntax in self.meta_syntaxes {
                    // check for stub
                    if self.in_stub.is_none() && meta_syntax.stub_begin.is_match(&s) {
                        log::trace!("stub start: '{}'", s);
                        // remove stub start
                        s = meta_syntax
                            .stub_begin
                            .replace(&s, |caps: &Captures| caps[1].to_string())
                            .to_string();

                        if s.trim().is_empty() && meta_syntax.stub_end.is_match(&s) {
                            // empty oneliner stubs are replaced by a newline
                            return Some(Ok(MetaString::Stub("\n".to_string())));
                        }

                        // save the syntax that started the current stub
                        self.in_stub = Some(meta_syntax);

                        if s.trim().is_empty() {
                            // only metadata, skip
                            return self.next();
                        }
                    }
                    // if the line matches stub_end and the saved syntax matches
                    // the start of the current meta syntax, return stub contents if any
                    if meta_syntax.stub_end.is_match(&s)
                        && self.in_stub.map(|r| r.stub_begin.as_str())
                            == Some(meta_syntax.stub_begin.as_str())
                    {
                        log::trace!("stub end: '{}'", s);
                        self.in_stub = None;
                        // remove stub end
                        s = meta_syntax
                            .stub_end
                            .replace(&s, |caps: &Captures| caps[1].to_string())
                            .to_string();
                        if s.trim().is_empty() {
                            // only metadata, skip
                            return self.next();
                        }
                        // return the stub contents
                        return Some(Ok(MetaString::Stub(s)));
                    }

                    // check for solution, skip solution begin/end markers
                    if meta_syntax.solution_file.is_match(&s) {
                        log::trace!("solution file marker");
                        return Some(Ok(MetaString::SolutionFileMarker));
                    } else if meta_syntax.solution_begin.is_match(&s) {
                        self.in_solution = true;
                        return self.next();
                    } else if meta_syntax.solution_end.is_match(&s) && self.in_solution {
                        self.in_solution = false;
                        return self.next();
                    } else if meta_syntax.hidden_file.is_match(&s) {
                        log::trace!("hidden file marker");
                        return Some(Ok(MetaString::HiddenFileMarker));
                    } else if meta_syntax.hidden_begin.is_match(&s) {
                        self.in_hidden = true;
                        return self.next();
                    } else if meta_syntax.hidden_end.is_match(&s) {
                        self.in_hidden = false;
                        return self.next();
                    }
                }
                // after processing the line with each meta syntax,
                // parse the current line accordingly
                if self.in_solution {
                    log::trace!("solution: '{}'", s);
                    Some(Ok(MetaString::Solution(s)))
                } else if self.in_stub.is_some() {
                    log::trace!("stub: '{}'", s);
                    Some(Ok(MetaString::Stub(s)))
                } else if self.in_hidden {
                    log::trace!("hidden: '{}'", s);
                    Some(Ok(MetaString::Hidden(s)))
                } else {
                    log::trace!("string: '{}'", s);
                    Some(Ok(MetaString::String(s)))
                }
            }
            Err(err) => Some(Err(TmcError::ReadLine(err))),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    impl MetaString {
        fn str(s: &str) -> Self {
            Self::String(s.to_string())
        }

        fn solution(s: &str) -> Self {
            Self::Solution(s.to_string())
        }

        fn stub(s: &str) -> Self {
            Self::Stub(s.to_string())
        }
    }

    #[test]
    fn parse_simple() {
        init();

        const JAVA_FILE: &str = r#"
public class JavaTestCase {
    // BEGIN SOLUTION
    public int foo() {
        return 3;
    }
    // END SOLUTION
}
"#;
        let expected: Vec<MetaString> = vec![
            MetaString::str("\n"),
            MetaString::str("public class JavaTestCase {\n"),
            MetaString::solution("    public int foo() {\n"),
            MetaString::solution("        return 3;\n"),
            MetaString::solution("    }\n"),
            MetaString::str("}\n"),
        ];

        let source = JAVA_FILE.as_bytes();
        let filter = MetaSyntaxParser::new(source, "java");
        let actual = filter.map(|l| l.unwrap()).collect::<Vec<MetaString>>();
        assert_eq!(expected, actual);
    }

    #[test]
    fn parse_solution() {
        init();

        const JAVA_FILE_SOLUTION: &str = r#"
/*    SOLUTION  FILE    */
public class JavaTestCase {
    public int foo() {
        return 3;
    }
}
"#;
        let expected: Vec<MetaString> = vec![
            MetaString::str("\n"),
            MetaString::SolutionFileMarker,
            MetaString::str("public class JavaTestCase {\n"),
            MetaString::str("    public int foo() {\n"),
            MetaString::str("        return 3;\n"),
            MetaString::str("    }\n"),
            MetaString::str("}\n"),
        ];

        let source = JAVA_FILE_SOLUTION.as_bytes();
        let filter = MetaSyntaxParser::new(source, "java");
        let actual = filter.map(|l| l.unwrap()).collect::<Vec<MetaString>>();
        assert_eq!(expected, actual);
    }

    #[test]
    fn parse_stubs() {
        init();

        const JAVA_FILE_STUB: &str = r#"
public class JavaTestCase {
    public int foo() {
        return 3;
        // STUB: return 0;
        /* STUB:
        stubs
        stubs
        */
    }
}
"#;

        let expected: Vec<MetaString> = vec![
            MetaString::str("\n"),
            MetaString::str("public class JavaTestCase {\n"),
            MetaString::str("    public int foo() {\n"),
            MetaString::str("        return 3;\n"),
            MetaString::stub("        return 0;\n"),
            MetaString::stub("        stubs\n"),
            MetaString::stub("        stubs\n"),
            MetaString::str("    }\n"),
            MetaString::str("}\n"),
        ];

        let source = JAVA_FILE_STUB.as_bytes();
        let filter = MetaSyntaxParser::new(source, "java");
        let actual = filter.map(|l| l.unwrap()).collect::<Vec<MetaString>>();
        assert_eq!(expected, actual);
    }

    #[test]
    fn stube() {
        init();

        const PYTHON_FILE_STUB: &str = r#"
# BEGIN SOLUTION
print("a")
# END SOLUTION
# KOMMENTTI
#STUB:class Kauppalista:
    #STUB:def __init__(self):
        #STUB:self.tuotteet = []
    #STUB:
        #STUB:def tuotteita(self):
            #STUB:return len(self.tuotteet)
    #STUB:
        #STUB:def lisaa(self, tuote: str, maara: int):
            #STUB:self.tuotteet.append((tuote, maara))
    #STUB:
        #STUB:def tuote(self, n: int):
            #STUB:return self.tuotteet[n - 1][0]
    #STUB:
        #STUB:def maara(self, n:int):
            #STUB:return self.uotteet[n - 1][1]
"#;

        let expected: Vec<MetaString> = vec![
            MetaString::str("\n"),
            MetaString::solution("print(\"a\")\n"),
            MetaString::str("# KOMMENTTI\n"),
            MetaString::stub("class Kauppalista:\n"),
            MetaString::stub("    def __init__(self):\n"),
            MetaString::stub("        self.tuotteet = []\n"),
            MetaString::stub("\n"),
            MetaString::stub("        def tuotteita(self):\n"),
            MetaString::stub("            return len(self.tuotteet)\n"),
            MetaString::stub("\n"),
            MetaString::stub("        def lisaa(self, tuote: str, maara: int):\n"),
            MetaString::stub("            self.tuotteet.append((tuote, maara))\n"),
            MetaString::stub("\n"),
            MetaString::stub("        def tuote(self, n: int):\n"),
            MetaString::stub("            return self.tuotteet[n - 1][0]\n"),
            MetaString::stub("\n"),
            MetaString::stub("        def maara(self, n:int):\n"),
            MetaString::stub("            return self.uotteet[n - 1][1]\n"),
        ];

        let source = PYTHON_FILE_STUB.as_bytes();
        let filter = MetaSyntaxParser::new(source, "py");
        let actual = filter.map(|l| l.unwrap()).collect::<Vec<MetaString>>();
        assert_eq!(expected, actual);
    }
}
