use super::{InterpPart, split_interpolation};

fn lit(s: &str) -> InterpPart {
    InterpPart::Lit(s.to_string())
}

fn expr(s: &str) -> InterpPart {
    InterpPart::Expr(s.to_string())
}

#[test]
fn plain_text_is_one_literal() {
    assert_eq!(split_interpolation("hello"), vec![lit("hello")]);
    assert_eq!(split_interpolation(""), Vec::<InterpPart>::new());
}

#[test]
fn splits_around_expressions() {
    assert_eq!(
        split_interpolation("a%{x}b"),
        vec![lit("a"), expr("x"), lit("b")]
    );
    assert_eq!(split_interpolation("%{x}%{y}"), vec![expr("x"), expr("y")]);
    assert_eq!(split_interpolation("%{x}"), vec![expr("x")]);
}

#[test]
fn braces_nest_by_depth() {
    assert_eq!(
        split_interpolation("v: %{ #{ 'a': 1 }.at:'a' }!"),
        vec![lit("v: "), expr(" #{ 'a': 1 }.at:'a' "), lit("!")]
    );
}

#[test]
fn unterminated_marker_is_literal() {
    assert_eq!(
        split_interpolation("a%{ never closed"),
        vec![lit("a%{ never closed")]
    );
    // A later complete marker still splits.
    assert_eq!(
        split_interpolation("a%{ open %{x}"),
        vec![lit("a%{ open "), expr("x")]
    );
}

#[test]
fn empty_expression_is_kept() {
    assert_eq!(
        split_interpolation("a%{}b"),
        vec![lit("a"), expr(""), lit("b")]
    );
}

#[test]
fn bare_percent_and_braces_are_literal() {
    assert_eq!(split_interpolation("100% {x}"), vec![lit("100% {x}")]);
    assert_eq!(split_interpolation("%"), vec![lit("%")]);
}
