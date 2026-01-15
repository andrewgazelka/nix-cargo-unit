use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct TestParser;

#[cfg(test)]
mod tests {
    use super::*;
    use pest::Parser;

    #[test]
    fn test_parse() {
        let result = TestParser::parse(Rule::ident, "hello");
        assert!(result.is_ok());
    }
}
