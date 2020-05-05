use lazy_static::lazy_static;
use log::debug;
use regex::Regex;
use std::io::{self, BufRead, BufReader, Read};

lazy_static! {
    static ref META_SYNTAXES_C: [MetaSyntax; 2] =
        [MetaSyntax::new("//", ""), MetaSyntax::new("/\\*", "\\*/")];
    static ref META_SYNTAXES_HTML: [MetaSyntax; 1] = [MetaSyntax::new("<!--", "-->")];
    static ref META_SYNTAXES_PY: [MetaSyntax; 1] = [MetaSyntax::new("#", "")];
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
    fn new(comment_start: &'static str, comment_end: &'static str) -> Self {
        let comment_start_pattern = format!("^\\s*{}\\s*", comment_start);
        let comment_end_pattern = format!("\\s*{}\\s*$", comment_end);
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
        let stub_begin = Regex::new(&format!("{}STUB", comment_start_pattern)).unwrap();
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
pub struct MetaSyntaxFilter<B: BufRead> {
    meta_syntaxes: &'static [MetaSyntax],
    reader: B,
}

impl<R: Read> MetaSyntaxFilter<BufReader<R>> {
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
        }
    }
}

impl<B: BufRead> Iterator for MetaSyntaxFilter<B> {
    type Item = Result<String, io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        // read lines until a non-skipped line is found or the reader is empty
        'next_line: loop {
            let mut s = String::new();
            match self.reader.read_line(&mut s) {
                // read 0 bytes = reader empty = iterator empty
                Ok(0) => return None,
                Ok(_) => {
                    // check line with each meta syntax
                    for meta_syntax in self.meta_syntaxes {
                        if meta_syntax.stub_begin.is_match(&s) {
                            // in STUB block
                            debug!("stub start: {}", s);
                            if meta_syntax.stub_end.is_match(&s) {
                                // oneliner
                                debug!("stub end: {}", s);
                                continue 'next_line;
                            }
                            // skip STUB block
                            loop {
                                match self.reader.read_line(&mut s) {
                                    Ok(0) => return None,
                                    Ok(_) => {
                                        if meta_syntax.stub_end.is_match(&s) {
                                            debug!("stub end: {}", s);
                                            continue 'next_line;
                                        }
                                        debug!("skip stub: {}", s);
                                    }
                                    Err(err) => return Some(Err(err)),
                                }
                            }
                        }
                        debug!("{}", meta_syntax.solution_file.as_str());
                        if meta_syntax.solution_file.is_match(&s)
                            | meta_syntax.solution_begin.is_match(&s)
                            | meta_syntax.solution_end.is_match(&s)
                        {
                            debug!("skip solution: {}", s);
                            // skip SOLUTION_FILE / SOLUTION_BEGIN / SOLUTION_END lines
                            continue 'next_line;
                        }
                    }
                    // not filtered by any syntax
                    debug!("OK: {}", s);
                    return Some(Ok(s));
                }
                Err(err) => return Some(Err(err)),
            }
        }
    }
}

pub struct TestDesc {}

pub struct ExerciseDesc {}

pub struct RunResult {}

pub struct ValidationResult {}

pub struct ExercisePackagingConfiguration {}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    const JAVA_FILE: &'static str = r#"
public class JavaTestCase {
    public int foo() {
        return 3;
    }
}
"#;

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

    #[test]
    fn nothing_to_filter() {
        init();
        let source = JAVA_FILE.as_bytes();
        let filter = MetaSyntaxFilter::new(source, "java");
        assert_eq!(filter.map(|l| l.unwrap()).collect::<String>(), JAVA_FILE);
    }

    #[test]
    fn filter_solution_markers() {
        init();
        let source = JAVA_FILE_SOLUTION.as_bytes();
        let filter = MetaSyntaxFilter::new(source, "java");
        assert_eq!(filter.map(|l| l.unwrap()).collect::<String>(), JAVA_FILE);
    }

    #[test]
    fn filter_stubs() {
        init();
        let source = JAVA_FILE_STUB.as_bytes();
        let filter = MetaSyntaxFilter::new(source, "java");
        assert_eq!(filter.map(|l| l.unwrap()).collect::<String>(), JAVA_FILE);
    }
}
