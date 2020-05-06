use lazy_static::lazy_static;
use log::debug;
use regex::{Captures, Regex};
use std::io::{self, BufRead, BufReader, Read};

lazy_static! {
    static ref META_SYNTAXES_C: [MetaSyntax; 2] = [
        MetaSyntax::new("//", None),
        MetaSyntax::new("/\\*", Some("\\*/"))
    ];
    static ref META_SYNTAXES_HTML: [MetaSyntax; 1] = [MetaSyntax::new("<!--", Some("-->"))];
    static ref META_SYNTAXES_PY: [MetaSyntax; 1] = [MetaSyntax::new("#", None)];
}

#[derive(Debug, PartialEq, Eq)]
pub enum MetaString {
    String(String),
    Stub(String),
    Solution(String),
}

impl MetaString {
    pub fn as_str<'a>(&'a self) -> &'a str {
        match self {
            Self::String(s) => &s,
            Self::Stub(s) => &s,
            Self::Solution(s) => &s,
        }
    }
}

#[derive(Debug)]
struct MetaSyntax {
    solution_file: Regex,
    solution_begin: Regex,
    solution_end: Regex,
    stub_begin: Regex,
    stub_end: Regex,
}

impl MetaSyntax {
    fn new(comment_start: &'static str, comment_end: Option<&'static str>) -> Self {
        let comment_start_pattern = format!("^(\\s*){}\\s*", comment_start);
        let comment_end_pattern = match comment_end {
            Some(s) => format!("(.*){}\\s*", s),
            None => "(.*)".to_string(),
        };
        let solution_file = Regex::new(&format!(
            "{}SOLUTION\\s+FILE{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let solution_begin = Regex::new(&format!(
            "{}BEGIN\\s+SOLUTION{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let solution_end = Regex::new(&format!(
            "{}END\\s+SOLUTION{}",
            comment_start_pattern, comment_end_pattern
        ))
        .unwrap();
        let stub_begin = Regex::new(&format!("{}STUB:\\s*", comment_start_pattern)).unwrap();
        let stub_end = Regex::new(&comment_end_pattern).unwrap();

        Self {
            solution_file,
            solution_begin,
            solution_end,
            stub_begin,
            stub_end,
        }
    }
}

#[derive(Debug)]
pub struct MetaSyntaxParser<B: BufRead> {
    meta_syntaxes: &'static [MetaSyntax],
    reader: B,
    in_stub: Option<&'static MetaSyntax>,
    in_solution: bool,
}

impl<R: Read> MetaSyntaxParser<BufReader<R>> {
    pub fn new(target: R, target_extension: &str) -> Self {
        let reader = BufReader::new(target);
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
        }
    }
}

impl<B: BufRead> Iterator for MetaSyntaxParser<B> {
    type Item = io::Result<MetaString>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut s = String::new();
        match self.reader.read_line(&mut s) {
            // read 0 bytes = reader empty = iterator empty
            Ok(0) => return None,
            Ok(_) => {
                // check line with each meta syntax
                for meta_syntax in self.meta_syntaxes {
                    // stub
                    if self.in_stub.is_none() && meta_syntax.stub_begin.is_match(&s) {
                        debug!("stub start: '{}'", s);
                        self.in_stub = Some(meta_syntax);
                        // remove stub start
                        s = meta_syntax
                            .stub_begin
                            .replace(&s, |caps: &Captures| format!("{}", &caps[1]))
                            .to_string();
                        debug!("parsed: '{}'", s);
                        if s.trim().is_empty() {
                            // only metadata
                            return self.next();
                        }
                    }
                    if meta_syntax.stub_end.is_match(&s)
                        && self.in_stub.map(|r| r.stub_begin.as_str())
                            == Some(meta_syntax.stub_begin.as_str())
                    {
                        debug!("stub end: '{}'", s);
                        self.in_stub = None;
                        s = meta_syntax
                            .stub_end
                            .replace(&s, |caps: &Captures| format!("{}", &caps[1]))
                            .to_string();
                        debug!("parsed: '{}'", s);
                        if s.trim().is_empty() {
                            // only metadata
                            return self.next();
                        }
                        return Some(Ok(MetaString::Stub(s)));
                    }

                    // solution
                    if meta_syntax.solution_file.is_match(&s) {
                        return self.next();
                    } else if meta_syntax.solution_begin.is_match(&s) {
                        self.in_solution = true;
                        return self.next();
                    } else if meta_syntax.solution_end.is_match(&s) && self.in_solution {
                        self.in_solution = false;
                        return self.next();
                    }
                }
                if self.in_solution {
                    debug!("solution: '{}'", s);
                    return Some(Ok(MetaString::Solution(s)));
                } else if self.in_stub.is_some() {
                    debug!("stub: '{}'", s);
                    return Some(Ok(MetaString::Stub(s)));
                } else {
                    debug!("string: '{}'", s);
                    return Some(Ok(MetaString::String(s)));
                }
            }
            Err(err) => return Some(Err(err)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
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

        const JAVA_FILE: &'static str = r#"
public class JavaTestCase {
    public int foo() {
        return 3;
    }
}
"#;
        let expected: Vec<MetaString> = vec![
            MetaString::str("\n"),
            MetaString::str("public class JavaTestCase {\n"),
            MetaString::str("    public int foo() {\n"),
            MetaString::str("        return 3;\n"),
            MetaString::str("    }\n"),
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

        const JAVA_FILE_SOLUTION: &'static str = r#"
/*    SOLUTION  FILE    */
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

        let source = JAVA_FILE_SOLUTION.as_bytes();
        let filter = MetaSyntaxParser::new(source, "java");
        let actual = filter.map(|l| l.unwrap()).collect::<Vec<MetaString>>();
        assert_eq!(expected, actual);
    }

    #[test]
    fn parse_stubs() {
        init();

        const JAVA_FILE_STUB: &'static str = r#"
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
}
