mod mock_externals;

use indoc::indoc;
use maplit::btreemap;
use permute::permute;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use mock_externals::MockExternal;
use polar_core::{
    error::*,
    events::*,
    messages::*,
    polar::{Polar, Query},
    sym, term,
    terms::*,
    traces::*,
    value, values,
};

fn polar() -> Polar {
    let mut p = Polar::new();
    p.set_ignore_no_allow_warning(true);
    p
}

fn no_results(
    _: u64,
    _: Term,
    _: Symbol,
    _: Option<Vec<Term>>,
    _: Option<BTreeMap<Symbol, Term>>,
) -> Option<Term> {
    None
}

fn print_messages(msg: &Message) {
    eprintln!("[{:?}] {}", msg.kind, msg.msg);
}

fn no_externals(_: u64, _: Term) {}

fn no_debug(_: &str) -> String {
    "".to_string()
}

type QueryResults = Vec<(HashMap<Symbol, Value>, Option<TraceResult>)>;

fn no_error_handler(e: PolarError) -> QueryResults {
    panic!("Query returned error: {}", e.to_string())
}

fn no_isa(_: Term, _: Symbol) -> bool {
    true
}

fn no_is_subspecializer(_: u64, _: Symbol, _: Symbol) -> bool {
    false
}

#[allow(clippy::too_many_arguments)]
fn query_results<F, G, H, I, J, K, L>(
    mut query: Query,
    mut external_call_handler: F,
    mut make_external_handler: H,
    mut external_isa_handler: I,
    mut external_is_subspecializer_handler: J,
    mut debug_handler: G,
    mut message_handler: K,
    mut error_handler: L,
) -> QueryResults
where
    F: FnMut(u64, Term, Symbol, Option<Vec<Term>>, Option<BTreeMap<Symbol, Term>>) -> Option<Term>,
    G: FnMut(&str) -> String,
    H: FnMut(u64, Term),
    I: FnMut(Term, Symbol) -> bool,
    J: FnMut(u64, Symbol, Symbol) -> bool,
    K: FnMut(&Message),
    L: FnMut(PolarError) -> QueryResults,
{
    let mut results = vec![];
    loop {
        let event = match query.next_event() {
            Err(e) => return error_handler(e),
            Ok(e) => e,
        };

        while let Some(msg) = query.next_message() {
            message_handler(&msg)
        }
        match event {
            QueryEvent::Done { .. } => break,
            QueryEvent::Result { bindings, trace } => {
                results.push((
                    bindings
                        .into_iter()
                        .map(|(k, v)| (k, v.value().clone()))
                        .collect(),
                    trace,
                ));
            }
            QueryEvent::ExternalCall {
                call_id,
                instance,
                attribute,
                args,
                kwargs,
            } => {
                query
                    .call_result(
                        call_id,
                        external_call_handler(call_id, instance, attribute, args, kwargs),
                    )
                    .unwrap();
            }
            QueryEvent::MakeExternal {
                instance_id,
                constructor,
            } => make_external_handler(instance_id, constructor),
            QueryEvent::ExternalIsa {
                call_id,
                instance,
                class_tag,
            } => query
                .question_result(call_id, external_isa_handler(instance, class_tag))
                .unwrap(),
            QueryEvent::ExternalIsSubSpecializer {
                call_id,
                instance_id,
                left_class_tag,
                right_class_tag,
            } => query
                .question_result(
                    call_id,
                    external_is_subspecializer_handler(
                        instance_id,
                        left_class_tag,
                        right_class_tag,
                    ),
                )
                .unwrap(),
            QueryEvent::Debug { ref message } => {
                query.debug_command(&debug_handler(message)).unwrap();
            }
            QueryEvent::ExternalOp {
                operator: Operator::Eq,
                call_id,
                args,
                ..
            } => query.question_result(call_id, args[0] == args[1]).unwrap(),
            _ => {}
        }
    }
    results
}

macro_rules! query_results {
    ($query:expr) => {
        query_results(
            $query,
            no_results,
            no_externals,
            no_isa,
            no_is_subspecializer,
            no_debug,
            print_messages,
            no_error_handler,
        )
    };
    ($query:expr, $external_call_handler:expr, $make_external_handler:expr, $debug_handler:expr) => {
        query_results(
            $query,
            $external_call_handler,
            $make_external_handler,
            no_isa,
            no_is_subspecializer,
            $debug_handler,
            print_messages,
            no_error_handler,
        )
    };
    ($query:expr, $external_call_handler:expr) => {
        query_results(
            $query,
            $external_call_handler,
            no_externals,
            no_isa,
            no_is_subspecializer,
            no_debug,
            print_messages,
            no_error_handler,
        )
    };
    ($query:expr, @msgs $message_handler:expr) => {
        query_results(
            $query,
            no_results,
            no_externals,
            no_isa,
            no_is_subspecializer,
            no_debug,
            $message_handler,
            no_error_handler,
        )
    };
    ($query:expr, @errs $error_handler:expr) => {
        query_results(
            $query,
            no_results,
            no_externals,
            no_isa,
            no_is_subspecializer,
            no_debug,
            print_messages,
            $error_handler,
        )
    };
}

fn query_results_with_externals(query: Query) -> (QueryResults, MockExternal) {
    let mock = RefCell::new(MockExternal::new());
    (
        query_results(
            query,
            |a, b, c, d, e| mock.borrow_mut().external_call(a, b, c, d, e),
            |a, b| mock.borrow_mut().make_external(a, b),
            |a, b| mock.borrow_mut().external_isa(a, b),
            |a, b, c| mock.borrow_mut().external_is_subspecializer(a, b, c),
            no_debug,
            print_messages,
            no_error_handler,
        ),
        mock.into_inner(),
    )
}

/// equality test for polar expressions that takes symmetric operators
/// into account, eg. a = b == b = a
fn commute_ops(u: &Value, v: &Value) -> bool {
    fn a2p(a: &[Term]) -> (&Value, &Value) {
        (a[0].value(), a[1].value())
    }
    match (u.as_expression(), v.as_expression()) {
        (
            Ok(Operation {
                operator: op_a,
                args: arg_a,
            }),
            Ok(Operation {
                operator: op_b,
                args: arg_b,
            }),
        ) if op_a == op_b && arg_a.len() == arg_b.len() => {
            let op = *op_a;
            if arg_a.len() == 2
                && (op == Operator::Unify
                    || op == Operator::Eq
                    || op == Operator::Neq
                    || op == Operator::And
                    || op == Operator::Or)
            {
                let (a, b) = (a2p(arg_a), a2p(arg_b));
                commute_ops(a.0, b.0) && commute_ops(a.1, b.1)
                    || commute_ops(a.0, b.1) && commute_ops(a.1, b.0)
            } else {
                arg_a
                    .iter()
                    .enumerate()
                    .all(|(i, x)| commute_ops(arg_b[i].value(), x.value()))
            }
        }
        _ => u == v,
    }
}

#[track_caller]
#[must_use = "test results need to be asserted"]
fn eval(p: &mut Polar, query_str: &str) -> bool {
    let q = p.new_query(query_str, false).unwrap();
    !query_results!(q).is_empty()
}

#[track_caller]
fn qeval(p: &mut Polar, query_str: &str) {
    assert!(eval(p, query_str));
}

#[track_caller]
#[must_use = "test results need to be asserted"]
fn null(p: &mut Polar, query_str: &str) -> bool {
    let q = p.new_query(query_str, false).unwrap();
    query_results!(q).is_empty()
}

#[track_caller]
fn qnull(p: &mut Polar, query_str: &str) {
    assert!(null(p, query_str));
}

#[track_caller]
fn qext(p: &mut Polar, query_str: &str, external_results: Vec<Value>, expected_len: usize) {
    let mut external_results = external_results.into_iter().map(Term::new_from_test);
    let q = p.new_query(query_str, false).unwrap();
    assert_eq!(
        query_results!(q, |_, _, _, _, _| external_results.next()).len(),
        expected_len
    );
}

#[track_caller]
#[must_use = "test results need to be asserted"]
fn var(p: &mut Polar, query_str: &str, var: &str) -> Vec<Value> {
    let q = p.new_query(query_str, false).unwrap();
    query_results!(q)
        .iter()
        .map(|(r, _)| &r[&sym!(var)])
        .cloned()
        .collect()
}

#[track_caller]
fn qvar(p: &mut Polar, query_str: &str, variable: &str, expected: Vec<Value>) {
    assert_eq!(var(p, query_str, variable), expected);
}

#[track_caller]
#[must_use = "test results need to be asserted"]
fn vars(p: &mut Polar, query_str: &str, vars: &[&str]) -> Vec<Vec<Value>> {
    let q = p.new_query(query_str, false).unwrap();
    query_results!(q)
        .iter()
        .map(|bindings| {
            vars.iter()
                .map(|&var| bindings.0.get(&Symbol(var.to_string())).unwrap().clone())
                .collect()
        })
        .collect()
}

#[track_caller]
fn qvars(p: &mut Polar, query_str: &str, variables: &[&str], expected: Vec<Vec<Value>>) {
    assert_eq!(vars(p, query_str, variables), expected);
}

#[track_caller]
fn _qruntime(p: &mut Polar, query_str: &str) -> ErrorKind {
    p.new_query(query_str, false)
        .unwrap()
        .next_event()
        .unwrap_err()
        .kind
}

macro_rules! qruntime {
    ($query:tt, $err:pat $(, $cond:expr)?) => {
        assert!(matches!(_qruntime(&mut polar(), $query), ErrorKind::Runtime($err) $(if $cond)?));
    };

    ($polar:expr, $query:tt, $err:pat $(, $cond:expr)?) => {
        assert!(matches!(_qruntime($polar, $query), ErrorKind::Runtime($err) $(if $cond)?));
    };
}

macro_rules! qparse {
    ($query:expr, $err:pat) => {
        assert!(matches!(
            polar().load_str($query).unwrap_err().kind,
            ErrorKind::Parse($err)
        ));
    };
}

type TestResult = Result<(), PolarError>;

/// Adapted from <http://web.cse.ohio-state.edu/~stiff.4/cse3521/prolog-resolution.html>
#[test]
fn test_functions() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(1);
           f(2);
           g(1);
           g(2);
           h(2);
           k(x) if f(x) and h(x) and g(x);"#,
    )?;
    qnull(&mut p, "k(1)");
    qeval(&mut p, "k(2)");
    qnull(&mut p, "k(3)");
    qvar(&mut p, "k(a)", "a", values![2]);
    Ok(())
}

/// Adapted from <http://web.cse.ohio-state.edu/~stiff.4/cse3521/prolog-resolution.html>
#[test]
fn test_jealous() -> TestResult {
    let p = polar();
    p.load_str(
        r#"loves("vincent", "mia");
           loves("marcellus", "mia");
           jealous(a, b) if loves(a, c) and loves(b, c);"#,
    )?;
    let q = p.new_query("jealous(who, of)", false)?;
    let results = query_results!(q);
    let jealous = |who: &str, of: &str| {
        assert!(
            &results.iter().any(|(r, _)| r
                == &HashMap::from_iter(vec![(sym!("who"), value!(who)), (sym!("of"), value!(of))])),
            "{} is not jealous of {} (but should be)",
            who,
            of
        );
    };
    assert_eq!(results.len(), 4);
    jealous("vincent", "vincent");
    jealous("vincent", "marcellus");
    jealous("marcellus", "vincent");
    jealous("marcellus", "marcellus");
    Ok(())
}

#[test]
fn test_trace() -> TestResult {
    let p = polar();
    p.load_str(
        r#"f(x) if x = 1 and x = 1;
           f(y) if y = 1;"#,
    )?;
    let q = p.new_query("f(1)", true)?;
    let results = query_results!(q);
    let trace = results[0].1.as_ref().unwrap();
    let expected = indoc!(
        r#"
        f(1) [
          f(x) if x = 1 and x = 1; [
              x = 1 []
              x = 1 []
          ]
        ]
        "#
    );
    assert_eq!(trace.formatted, expected);
    let trace = results[1].1.as_ref().unwrap();
    let expected = indoc!(
        r#"
        f(1) [
          f(y) if y = 1; [
              y = 1 []
          ]
        ]
        "#
    );
    assert_eq!(trace.formatted, expected);
    Ok(())
}

#[test]
fn test_nested_rule() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x) if g(x);
           g(x) if h(x);
           h(2);
           g(x) if j(x);
           j(4);"#,
    )?;
    qeval(&mut p, "f(2)");
    qnull(&mut p, "f(3)");
    qeval(&mut p, "f(4)");
    qeval(&mut p, "j(4)");
    Ok(())
}

/// A functions permutation that is known to fail.
#[test]
fn test_bad_functions() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(2);
           f(1);
           g(1);
           g(2);
           h(2);
           k(x) if f(x) and h(x) and g(x);"#,
    )?;
    qvar(&mut p, "k(a)", "a", values![2]);
    Ok(())
}

#[test]
fn test_functions_reorder() -> TestResult {
    // TODO (dhatch): Reorder f(x), h(x), g(x)
    let parts = vec![
        "f(1)",
        "f(2)",
        "g(1)",
        "g(2)",
        "h(2)",
        "k(x) if f(x) and g(x) and h(x)",
    ];

    for (i, permutation) in permute(parts).into_iter().enumerate() {
        let mut p = polar();

        let mut joined = permutation.join(";");
        joined.push(';');
        p.load_str(&joined)?;

        assert!(
            null(&mut p, "k(1)"),
            "k(1) was true for permutation {:?}",
            &permutation
        );
        assert!(
            eval(&mut p, "k(2)"),
            "k(2) failed for permutation {:?}",
            &permutation
        );
        assert_eq!(
            var(&mut p, "k(a)", "a"),
            values![2],
            "k(a) failed for permutation {:?}",
            &permutation
        );

        println!("permute: {}", i);
    }
    Ok(())
}

#[test]
fn test_results() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"foo(1);
           foo(2);
           foo(3);"#,
    )?;
    qvar(&mut p, "foo(a)", "a", values![1, 2, 3]);
    Ok(())
}

#[test]
fn test_result_permutations() -> TestResult {
    let parts = vec![
        (1, "foo(1)"),
        (2, "foo(2)"),
        (3, "foo(3)"),
        (4, "foo(4)"),
        (5, "foo(5)"),
    ];
    for permutation in permute(parts).into_iter() {
        eprintln!("{:?}", permutation);
        let mut p = polar();
        let (results, rules): (Vec<_>, Vec<_>) = permutation.into_iter().unzip();
        p.load_str(&format!("{};", rules.join(";")))?;
        qvar(
            &mut p,
            "foo(a)",
            "a",
            results.into_iter().map(|v| value!(v)).collect::<Vec<_>>(),
        );
    }
    Ok(())
}

#[test]
fn test_multi_arg_method_ordering() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"bar(2, 1);
           bar(1, 1);
           bar(1, 2);
           bar(2, 2);"#,
    )?;
    qvars(
        &mut p,
        "bar(a, b)",
        &["a", "b"],
        values![[2, 1], [1, 1], [1, 2], [2, 2]],
    );
    Ok(())
}

#[test]
fn test_no_applicable_rules() -> TestResult {
    let mut p = polar();
    qnull(&mut p, "f()");
    p.load_str("f(_);")?;
    qnull(&mut p, "f()");
    Ok(())
}

/// From Aït-Kaci's WAM tutorial (1999), page 34.
#[test]
fn test_ait_kaci_34() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"a() if b(x) and c(x);
           b(x) if e(x);
           c(1);
           e(x) if f(x);
           e(x) if g(x);
           f(2);
           g(1);"#,
    )?;
    qeval(&mut p, "a()");
    Ok(())
}

#[test]
fn test_constants() -> TestResult {
    let mut p = polar();
    {
        let mut kb = p.kb.write().unwrap();
        kb.register_constant(sym!("one"), term!(1))?;
        kb.register_constant(sym!("two"), term!(2))?;
        kb.register_constant(sym!("three"), term!(3))?;
    }
    p.load_str(
        r#"one(x) if one = one and one = x and x < two;
           two(x) if one < x and two = two and two = x and two < three;
           three(x) if three = three and three = x;"#,
    )?;
    qeval(&mut p, "one(1)");
    qnull(&mut p, "two(1)");
    qeval(&mut p, "two(2)");
    qnull(&mut p, "three(2)");
    qeval(&mut p, "three(3)");
    Ok(())
}

#[test]
fn test_not() -> TestResult {
    let mut p = polar();
    p.load_str("odd(1); even(2);")?;
    qeval(&mut p, "odd(1)");
    qnull(&mut p, "not odd(1)");
    qnull(&mut p, "even(1)");
    qeval(&mut p, "not even(1)");
    qnull(&mut p, "odd(2)");
    qeval(&mut p, "not odd(2)");
    qeval(&mut p, "even(2)");
    qnull(&mut p, "not even(2)");
    qnull(&mut p, "even(3)");
    qeval(&mut p, "not even(3)");

    p.clear_rules();

    p.load_str(
        r#"f(x) if not a(x);
           a(1);
           b(2);
           g(x) if not (a(x) or b(x));"#,
    )?;

    qnull(&mut p, "f(1)");
    qeval(&mut p, "f(2)");

    qnull(&mut p, "g(1)");
    qnull(&mut p, "g(2)");
    qeval(&mut p, "g(3)");
    qeval(&mut p, "g(x) and x=3");
    qeval(&mut p, "x=3 and g(x)");

    p.clear_rules();

    p.load_str("h(x) if not (not (x = 1 or x = 3) or x = 3);")?;
    qeval(&mut p, "h(1)");
    qnull(&mut p, "h(2)");
    qnull(&mut p, "h(3)");

    qeval(&mut p, "d = {x: 1} and not d.x = 2");

    p.clear_rules();

    // Negate And with unbound variable.
    p.load_str("i(x,y) if not (y = 2 and x = 1);")?;
    qvar(&mut p, "i(2,y)", "y", values![sym!("y")]);

    p.clear_rules();

    // Negate Or with unbound variable.
    p.load_str("j(x,y) if not (y = 2 or x = 1);")?;
    qeval(&mut p, "j(2, y)");
    Ok(())
}

#[test]
fn test_and() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(1);
           f(2);"#,
    )?;
    qeval(&mut p, "f(1) and f(2)");
    qnull(&mut p, "f(1) and f(2) and f(3)");
    Ok(())
}

#[test]
fn test_equality() {
    let mut p = polar();
    qeval(&mut p, "1 = 1");
    qnull(&mut p, "1 = 2");
}

#[test]
fn test_lookup() {
    qeval(&mut polar(), "{x: 1}.x = 1");
}

#[test]
fn test_instance_lookup() {
    // Q: Not sure if this should be allowed? I can't get (new a{x: 1}).x to parse, but that might
    // be the only thing we should permit
    qext(&mut polar(), "new a(x: 1).x = 1", values![1], 1);
}

/// Adapted from <http://web.cse.ohio-state.edu/~stiff.4/cse3521/prolog-resolution.html>
#[test]
fn test_retries() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(1);
           f(2);
           g(1);
           g(2);
           h(2);
           k(x) if f(x) and h(x) and g(x);
           k(3);"#,
    )?;
    qnull(&mut p, "k(1)");
    qeval(&mut p, "k(2)");
    qvar(&mut p, "k(a)", "a", values![2, 3]);
    qeval(&mut p, "k(3)");
    Ok(())
}

#[test]
fn test_two_rule_bodies() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x) if x = y and g(y);
           g(y) if y = 1;"#,
    )?;
    qvar(&mut p, "f(x)", "x", values![1]);
    Ok(())
}

#[test]
fn test_two_rule_bodies_not_nested() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x) if a(x);
           f(1);
           a(_x) if false;"#,
    )?;
    qvar(&mut p, "f(x)", "x", values![1]);
    Ok(())
}

#[test]
fn test_two_rule_bodies_nested() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x) if a(x);
           f(1);
           a(x) if g(x);
           g(_x) if false;"#,
    )?;
    qvar(&mut p, "f(x)", "x", values![1]);
    Ok(())
}

#[test]
fn test_unify_and() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x, y) if a(x) and y = 2;
           a(1);
           a(3);"#,
    )?;
    qvar(&mut p, "f(x, y)", "x", values![1, 3]);
    qvar(&mut p, "f(x, y)", "y", values![2, 2]);
    Ok(())
}

#[test]
fn test_symbol_lookup() {
    let mut p = polar();
    qvar(&mut p, "{x: 1}.x = res", "res", values![1]);
    qvar(&mut p, "{x: 1} = d and d.x = res", "res", values![1]);
}

#[test]
fn test_or() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(x) if a(x) or b(x);
           a(1);
           b(3);"#,
    )?;
    qvar(&mut p, "f(x)", "x", values![1, 3]);
    qeval(&mut p, "f(1)");
    qnull(&mut p, "f(2)");
    qeval(&mut p, "f(3)");

    p.clear_rules();

    p.load_str(
        r#"g(x) if a(x) or b(x) or c(x);
           a(1);
           b(3);
           c(5);"#,
    )?;
    qvar(&mut p, "g(x)", "x", values![1, 3, 5]);
    qeval(&mut p, "g(1)");
    qnull(&mut p, "g(2)");
    qeval(&mut p, "g(3)");
    qeval(&mut p, "g(5)");
    Ok(())
}

#[test]
fn test_dict_specializers() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f({x: 1});
           g(_: {x: 1});"#,
    )?;
    // Test unifying dicts against our rules.
    qeval(&mut p, "f({x: 1})");
    qnull(&mut p, "f({x: 1, y: 2})");
    qnull(&mut p, "f(1)");
    qnull(&mut p, "f({})");
    qnull(&mut p, "f({x: 2})");
    qnull(&mut p, "f({y: 1})");

    qeval(&mut p, "g({x: 1})");
    qeval(&mut p, "g({x: 1, y: 2})");
    qnull(&mut p, "g(1)");
    qnull(&mut p, "g({})");
    qnull(&mut p, "g({x: 2})");
    qnull(&mut p, "g({y: 1})");

    // Test unifying & isa-ing instances against our rules.
    qnull(&mut p, "f(new a(x: 1))");
    qext(&mut p, "g(new a(x: 1))", values![1, 1], 1);
    qnull(&mut p, "f(new a())");
    qnull(&mut p, "f(new a(x: {}))");
    qext(&mut p, "g(new a(x: 2))", values![2, 2], 0);
    qext(&mut p, "g(new a(y: 2, x: 1))", values![1, 1], 1);
    Ok(())
}

#[test]
fn test_non_instance_specializers() -> TestResult {
    let mut p = polar();
    p.load_str("f(x: 1) if x = 1;")?;
    qeval(&mut p, "f(1)");
    qnull(&mut p, "f(2)");

    p.clear_rules();

    p.load_str("g(x: 1, y: [x]) if y = [1];")?;
    qeval(&mut p, "g(1, [1])");
    qnull(&mut p, "g(1, [2])");

    p.clear_rules();

    p.load_str("h(x: {y: y}, x.y) if y = 1;")?;
    qeval(&mut p, "h({y: 1}, 1)");
    qnull(&mut p, "h({y: 1}, 2)");
    Ok(())
}

#[test]
#[allow(clippy::unnecessary_wraps)]
fn test_bindings() -> TestResult {
    let mut p = polar();

    // 0-cycle, aka ground.
    qvar(&mut p, "x=1", "x", values![1]);

    // 1-cycle is dropped.
    qeval(&mut p, "x=x");

    // 2-cycle.
    qvars(
        &mut p,
        "x=y and y=x",
        &["x", "y"],
        values![[sym!("y"), sym!("x")]],
    );

    // 3-cycle, 3 ways.
    qvars(
        &mut p,
        "x=y and y=z",
        &["x", "y", "z"],
        values![[sym!("z"), sym!("x"), sym!("y")]],
    );
    qvars(
        &mut p,
        "x=y and z=x",
        &["x", "y", "z"],
        values![[sym!("y"), sym!("z"), sym!("x")]],
    );

    // 4-cycle, 3 ways.
    qvars(
        &mut p,
        "x=y and y=z and z=w and w=x",
        &["x", "y", "z", "w"],
        values![[sym!("w"), sym!("x"), sym!("y"), sym!("z")]],
    );
    qvars(
        &mut p,
        "x=y and y=z and w=z and w=x",
        &["x", "y", "z", "w"],
        values![[sym!("w"), sym!("x"), sym!("y"), sym!("z")]],
    );
    qvars(
        &mut p,
        "x=y and w=z and z=x",
        &["x", "y", "z", "w"],
        values![[sym!("y"), sym!("z"), sym!("w"), sym!("x")]],
    );

    // Don't create sub-cycles.
    qvars(
        &mut p,
        "x=y and y=z and z=w and w=x and y=x",
        &["x", "y", "z", "w"],
        values![[sym!("w"), sym!("x"), sym!("y"), sym!("z")]],
    );

    // 6-cycle, 2 ways.
    qvars(
        &mut p,
        "x=y and y=z and z=w and w=v and v=u",
        &["x", "y", "z", "w", "v", "u"],
        values![[
            sym!("u"),
            sym!("x"),
            sym!("y"),
            sym!("z"),
            sym!("w"),
            sym!("v")
        ]],
    );
    qvars(
        &mut p,
        "x=y and y=z and w=v and v=u and u=x",
        &["x", "y", "z", "w", "v", "u"],
        values![[
            sym!("z"),
            sym!("u"),
            sym!("y"),
            sym!("x"),
            sym!("w"),
            sym!("v")
        ]],
    );

    Ok(())
}

#[test]
fn test_lookup_derefs() -> TestResult {
    let p = polar();
    p.load_str(
        r#"f(x) if x = y and g(y);
           g(y) if new Foo().get(y) = y;"#,
    )?;
    let q = p.new_query("f(1)", false)?;
    let mut foo_lookups = vec![term!(1)];
    let mock_foo = |_, _, _, args: Option<Vec<Term>>, _| {
        // check the argument is bound to an integer
        assert!(matches!(args.unwrap()[0].value(), Value::Number(_)));
        foo_lookups.pop()
    };

    let results = query_results!(q, mock_foo);
    assert!(foo_lookups.is_empty());
    assert_eq!(results.len(), 1);

    let mut foo_lookups = vec![term!(1)];
    let mock_foo = |_, _, _, args: Option<Vec<Term>>, _| {
        assert!(matches!(args.unwrap()[0].value(), Value::Number(_)));
        foo_lookups.pop()
    };
    let q = p.new_query("f(2)", false)?;
    let results = query_results!(q, mock_foo);
    assert!(results.is_empty());
    Ok(())
}

/// Test that rules are executed in the correct order.
#[test]
fn test_rule_order() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"a("foo");
           a("bar");
           a("baz");"#,
    )?;
    qvar(&mut p, "a(x)", "x", values!["foo", "bar", "baz"]);
    Ok(())
}

#[test]
fn test_load_str_with_query() -> TestResult {
    let p = polar();
    p.load_str(
        r#"f(1);
           f(2);
           ?= f(1);
           ?= not f(3);"#,
    )?;
    while let Some(q) = p.next_inline_query(false) {
        assert_eq!(query_results!(q).len(), 1);
    }
    Ok(())
}

/// Test using a constructor with positional + kwargs.
#[test]
fn test_make_external() -> TestResult {
    let q = polar().new_query("x = new Bar(1, a: 2, b: 3)", false)?;
    let mock_make_bar = |_, constructor: Term| match constructor.value() {
        Value::Call(Call {
            name,
            args,
            kwargs: Some(kwargs),
        }) if name == &sym!("Bar")
            && args == &vec![term!(1)]
            && kwargs == &btreemap! {sym!("a") => term!(2), sym!("b") => term!(3)} => {}
        _ => panic!("Expected call with args and kwargs"),
    };
    let results = query_results!(q, no_results, mock_make_bar, no_debug);
    assert_eq!(results.len(), 1);
    Ok(())
}

/// Test external call with positional + kwargs.
#[test]
fn test_external_call() -> TestResult {
    let p = polar();
    p.register_constant(sym!("Foo"), term!(true))?;
    let mut foo_lookups = vec![term!(1)];

    let q = p.new_query("(new Foo()).bar(1, a: 2, b: 3) = 1", false)?;

    let mock_foo_lookup =
        |_, _, _, args: Option<Vec<Term>>, kwargs: Option<BTreeMap<Symbol, Term>>| {
            assert_eq!(args.unwrap()[0], term!(1));
            assert_eq!(
                kwargs.unwrap(),
                btreemap! {sym!("a") => term!(2), sym!("b") => term!(3)}
            );
            foo_lookups.pop()
        };
    let results = query_results!(q, mock_foo_lookup);
    assert_eq!(results.len(), 1);
    assert!(foo_lookups.is_empty());
    Ok(())
}
#[test]
#[ignore] // ignore because this take a LONG time (could consider lowering the goal limit)
#[should_panic(expected = "Goal count exceeded! MAX_EXECUTED_GOALS = 10000")]
fn test_infinite_loop() {
    let mut p = polar();
    p.load_str("f(x) if f(x);").unwrap();
    qeval(&mut p, "f(1)");
}

#[test]
fn test_comparisons() -> TestResult {
    let mut p = polar();

    // <
    p.load_str("lt(x, y) if x < y;")?;
    qnull(&mut p, "lt(1,1)");
    qeval(&mut p, "lt(1,2)");
    qnull(&mut p, "lt(2,1)");
    qnull(&mut p, "lt(+1,-1)");
    qeval(&mut p, "lt(-1,+1)");
    qnull(&mut p, "lt(-1,-1)");
    qeval(&mut p, "lt(-2,-1)");
    qeval(&mut p, "lt(1019,1e19)");
    qnull(&mut p, "lt(1e19,1019)");
    qnull(&mut p, "lt(9007199254740992,9007199254740992)"); // identical
    qnull(&mut p, "lt(9007199254740992,9007199254740992.0)"); // equal
    qnull(&mut p, "lt(9007199254740992,9007199254740993.0)"); // indistinguishable
    qeval(&mut p, "lt(9007199254740992,9007199254740994.0)"); // distinguishable
    qeval(&mut p, "lt(\"aa\",\"ab\")");
    qnull(&mut p, "lt(\"aa\",\"aa\")");

    p.clear_rules();

    // <=
    p.load_str("leq(x, y) if x <= y;")?;
    qeval(&mut p, "leq(1,1)");
    qeval(&mut p, "leq(1,2)");
    qnull(&mut p, "leq(2,1)");
    qnull(&mut p, "leq(+1,-1)");
    qeval(&mut p, "leq(-1,+1)");
    qeval(&mut p, "leq(-1,-1)");
    qeval(&mut p, "leq(-2,-1)");
    qeval(&mut p, "leq(\"aa\",\"aa\")");
    qeval(&mut p, "leq(\"aa\",\"ab\")");
    qnull(&mut p, "leq(\"ab\",\"aa\")");

    p.clear_rules();

    // >
    p.load_str("gt(x, y) if x > y;")?;
    qnull(&mut p, "gt(1,1)");
    qnull(&mut p, "gt(1,2)");
    qeval(&mut p, "gt(2,1)");
    qeval(&mut p, "gt(+1,-1)");
    qnull(&mut p, "gt(-1,+1)");
    qnull(&mut p, "gt(-1,-1)");
    qeval(&mut p, "gt(-1,-2)");
    qeval(&mut p, "gt(\"ab\",\"aa\")");
    qnull(&mut p, "gt(\"aa\",\"aa\")");

    p.clear_rules();

    // >=
    p.load_str("geq(x, y) if x >= y;")?;
    qeval(&mut p, "geq(1,1)");
    qnull(&mut p, "geq(1,2)");
    qeval(&mut p, "geq(2,1)");
    qeval(&mut p, "geq(2,1)");
    qeval(&mut p, "geq(+1,-1)");
    qnull(&mut p, "geq(-1,+1)");
    qeval(&mut p, "geq(-1,-1)");
    qeval(&mut p, "geq(-1,-1.0)");
    qeval(&mut p, "geq(\"ab\",\"aa\")");
    qeval(&mut p, "geq(\"aa\",\"aa\")");

    p.clear_rules();

    // ==
    p.load_str("eq(x, y) if x == y;")?;
    qeval(&mut p, "eq(1,1)");
    qnull(&mut p, "eq(1,2)");
    qnull(&mut p, "eq(2,1)");
    qnull(&mut p, "eq(-1,+1)");
    qeval(&mut p, "eq(-1,-1)");
    qeval(&mut p, "eq(-1,-1.0)");
    qnull(&mut p, "eq(1019,1e19)");
    qnull(&mut p, "eq(1e19,1019)");
    qeval(&mut p, "eq(9007199254740992,9007199254740992)"); // identical
    qeval(&mut p, "eq(9007199254740992,9007199254740992.0)"); // equal
    qeval(&mut p, "eq(9007199254740992,9007199254740993.0)"); // indistinguishable
    qnull(&mut p, "eq(9007199254740992,9007199254740994.0)"); // distinguishable
    qeval(&mut p, "eq(\"aa\", \"aa\")");
    qnull(&mut p, "eq(\"ab\", \"aa\")");
    qeval(&mut p, "eq(bob, bob)");

    p.clear_rules();

    // !=
    p.load_str("neq(x, y) if x != y;")?;
    qnull(&mut p, "neq(1,1)");
    qeval(&mut p, "neq(1,2)");
    qeval(&mut p, "neq(2,1)");
    qeval(&mut p, "neq(-1,+1)");
    qnull(&mut p, "neq(-1,-1)");
    qnull(&mut p, "neq(-1,-1.0)");
    qnull(&mut p, "neq(\"aa\", \"aa\")");
    qeval(&mut p, "neq(\"ab\", \"aa\")");

    qeval(&mut p, "1.0 == 1");
    qeval(&mut p, "0.99 < 1");
    qeval(&mut p, "1.0 <= 1");
    qeval(&mut p, "1 == 1");
    qeval(&mut p, "0.0 == 0");

    qeval(&mut p, "x == y and x = 1 and y = 1");
    qnull(&mut p, "x == y and x = 1 and y = 2");
    Ok(())
}

#[test]
fn test_modulo_and_remainder() {
    let mut p = polar();
    qeval(&mut p, "1 mod 1 == 0");
    qeval(&mut p, "1 rem 1 == 0");
    qeval(&mut p, "1 mod -1 == 0");
    qeval(&mut p, "1 rem -1 == 0");
    qeval(&mut p, "0 mod 1 == 0");
    qeval(&mut p, "0 rem 1 == 0");
    qeval(&mut p, "0 mod -1 == 0");
    qeval(&mut p, "0 rem -1 == 0");
    let res = var(&mut p, "1 mod 0.0 = x", "x")[0].clone();
    if let Value::Number(Numeric::Float(x)) = res {
        assert!(x.is_nan());
    } else {
        panic!();
    }
    let res = var(&mut p, "1 rem 0.0 = x", "x")[0].clone();
    if let Value::Number(Numeric::Float(x)) = res {
        assert!(x.is_nan());
    } else {
        panic!();
    }

    // From http://www.lispworks.com/documentation/lw50/CLHS/Body/f_mod_r.htm.
    qeval(&mut p, "-1 rem 5 == -1");
    qeval(&mut p, "-1 mod 5 == 4");
    qeval(&mut p, "13 mod 4 == 1");
    qeval(&mut p, "13 rem 4 == 1");
    qeval(&mut p, "-13 mod 4 == 3");
    qeval(&mut p, "-13 rem 4 == -1");
    qeval(&mut p, "13 mod -4 == -3");
    qeval(&mut p, "13 rem -4 == 1");
    qeval(&mut p, "-13 mod -4 == -1");
    qeval(&mut p, "-13 rem -4 == -1");
    qeval(&mut p, "13.4 mod 1 == 0.40000000000000036");
    qeval(&mut p, "13.4 rem 1 == 0.40000000000000036");
    qeval(&mut p, "-13.4 mod 1 == 0.5999999999999996");
    qeval(&mut p, "-13.4 rem 1 == -0.40000000000000036");
}

#[test]
fn test_arithmetic() -> TestResult {
    let mut p = polar();
    qeval(&mut p, "1 + 1 == 2");
    qeval(&mut p, "1 + 1 < 3 and 1 + 1 > 1");
    qeval(&mut p, "2 - 1 == 1");
    qeval(&mut p, "1 - 2 == -1");
    qeval(&mut p, "1.23 - 3.21 == -1.98");
    qeval(&mut p, "2 * 3 == 6");
    qeval(&mut p, "6 / 2 == 3");
    qeval(&mut p, "2 / 6 == 0.3333333333333333");

    p.load_str(
        r#"even(0) if cut;
           even(x) if x > 0 and odd(x - 1);
           odd(1) if cut;
           odd(x) if x > 0 and even(x - 1);"#,
    )?;

    qeval(&mut p, "even(0)");
    qnull(&mut p, "even(1)");
    qeval(&mut p, "even(2)");
    qnull(&mut p, "even(3)");
    qeval(&mut p, "even(4)");

    qnull(&mut p, "odd(0)");
    qeval(&mut p, "odd(1)");
    qnull(&mut p, "odd(2)");
    qeval(&mut p, "odd(3)");
    qnull(&mut p, "odd(4)");

    qruntime!(
        "9223372036854775807 + 1 > 0",
        RuntimeError::ArithmeticError { .. }
    );
    qruntime!(
        "-9223372036854775807 - 2 < 0",
        RuntimeError::ArithmeticError { .. }
    );

    // x / 0 = ∞
    qvar(&mut p, "x=1/0", "x", values![f64::INFINITY]);
    qeval(&mut p, "1/0 = 2/0");
    qnull(&mut p, "1/0 < 0");
    qeval(&mut p, "1/0 > 0");
    qeval(&mut p, "1/0 > 1e100");
    Ok(())
}

#[test]
fn test_debug_break_on_error() -> TestResult {
    let p = polar();
    p.load_str("foo() if debug() and 1 < \"2\" and 1 < 2;")?;
    let mut call_num = 0;
    let debug_handler = |s: &str| {
        let rt = match call_num {
            0 => {
                let expected = indoc!(
                    r#"
                    QUERY: debug(), BINDINGS: {}

                    001: foo() if debug() and 1 < "2" and 1 < 2;
                                  ^
                    "#
                );
                assert_eq!(s, expected);
                "error"
            }
            1 => {
                let expected = indoc!(
                    r#"
                    QUERY: 1 < "2", BINDINGS: {}

                    001: foo() if debug() and 1 < "2" and 1 < 2;
                                              ^

                    ERROR: Not supported: 1 < "2"
                    "#
                );
                assert_eq!(s, expected);
                "c"
            }
            _ => panic!("Too many calls!"),
        };
        call_num += 1;
        rt.to_string()
    };
    let results = query_results(
        p.new_query("not foo()", false)?,
        no_results,
        no_externals,
        no_isa,
        no_is_subspecializer,
        debug_handler,
        print_messages,
        |_| Vec::new(),
    );
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn test_debug_temp_var() -> TestResult {
    let p = polar();
    p.load_str("foo(a, aa) if a < 10 and debug() and aa < a;")?;
    let mut call_num = 0;
    let debug_handler = |s: &str| {
        let rt = match call_num {
            0 => {
                let expected = indoc!(
                    r#"
                    QUERY: debug(), BINDINGS: {}

                    001: foo(a, aa) if a < 10 and debug() and aa < a;
                                                  ^
                    "#
                );
                assert_eq!(s, expected);
                "var a"
            }
            1 => {
                assert_eq!(s, "a@_a_3 = 5");
                "var aa"
            }
            2 => {
                assert_eq!(s, "aa@_aa_4 = 3");
                "q"
            }
            _ => panic!("Too many calls: {}", s),
        };
        call_num += 1;
        rt.to_string()
    };

    let q = p.new_query("foo(5, 3)", false)?;
    let _results = query_results!(q, no_results, no_externals, debug_handler);
    Ok(())
}

#[test]
fn test_debug() -> TestResult {
    let p = polar();
    p.load_str(indoc!(
        r#"a() if debug("a") and b() and c() and d();
           b();
           c() if debug("c");
           d();"#
    ))?;

    let mut call_num = 0;
    let debug_handler = |s: &str| {
        let rt = match call_num {
            0 => {
                let expected = indoc!(
                    r#"
                    QUERY: debug(), BINDINGS: {}

                    001: a() if debug("a") and b() and c() and d();
                                ^
                    002: b();
                    003: c() if debug("c");
                    004: d();
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            1 => {
                let expected = indoc!(
                    r#"
                    QUERY: b(), BINDINGS: {}

                    001: a() if debug("a") and b() and c() and d();
                                               ^
                    002: b();
                    003: c() if debug("c");
                    004: d();
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            2 => {
                let expected = indoc!(
                    r#"
                    QUERY: c(), BINDINGS: {}

                    001: a() if debug("a") and b() and c() and d();
                                                       ^
                    002: b();
                    003: c() if debug("c");
                    004: d();
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            3 => {
                let expected = indoc!(
                    r#"
                    QUERY: debug(), BINDINGS: {}

                    001: a() if debug("a") and b() and c() and d();
                    002: b();
                    003: c() if debug("c");
                                ^
                    004: d();
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            4 => {
                let expected = indoc!(
                    r#"
                    QUERY: d(), BINDINGS: {}

                    001: a() if debug("a") and b() and c() and d();
                                                               ^
                    002: b();
                    003: c() if debug("c");
                    004: d();
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            _ => panic!("Too many calls!"),
        };
        call_num += 1;
        rt.to_string()
    };

    let q = p.new_query("a()", false)?;
    let _results = query_results!(q, no_results, no_externals, debug_handler);

    let p = polar();
    p.load_str(indoc!(
        r#"a() if debug() and b() and c() and d();
           a() if 5 = 5;
           b() if 1 = 1 and 2 = 2;
           c() if 3 = 3 and 4 = 4;
           d();"#
    ))?;

    let mut call_num = 0;
    let debug_handler = |s: &str| {
        let rt = match call_num {
            0 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: debug(), BINDINGS: {}");
                "step"
            }
            1 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: b(), BINDINGS: {}");
                "step"
            }
            2 => {
                assert_eq!(
                    s.lines().next().unwrap(),
                    "QUERY: 1 = 1 and 2 = 2, BINDINGS: {}"
                );
                "out"
            }
            3 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: c(), BINDINGS: {}");
                "step"
            }
            4 => {
                assert_eq!(
                    s.lines().next().unwrap(),
                    "QUERY: 3 = 3 and 4 = 4, BINDINGS: {}"
                );
                "step"
            }
            5 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: 3 = 3, BINDINGS: {}");
                "out"
            }
            6 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: d(), BINDINGS: {}");
                "over"
            }
            7 => {
                assert_eq!(s.lines().next().unwrap(), "QUERY: 5 = 5, BINDINGS: {}");
                "c"
            }
            _ => panic!("Too many calls: {}", s),
        };
        call_num += 1;
        rt.to_string()
    };
    let q = p.new_query("a()", false)?;
    let _results = query_results!(q, no_results, no_externals, debug_handler);
    Ok(())
}

#[test]
fn test_debug_in_inverter() {
    let polar = polar();
    polar.load_str("a() if not debug();").unwrap();
    let mut call_num = 0;
    let debug_handler = |s: &str| {
        let rt = match call_num {
            0 => {
                let expected = indoc!(
                    r#"
                    QUERY: debug(), BINDINGS: {}

                    001: a() if not debug();
                                    ^
                    "#
                );
                assert_eq!(s, expected);
                "over"
            }
            _ => panic!("Too many calls: {}", s),
        };
        call_num += 1;
        rt.to_string()
    };
    let query = polar.new_query("a()", false).unwrap();
    let _results = query_results!(query, no_results, no_externals, debug_handler);
}

#[test]
fn test_anonymous_vars() {
    let mut p = polar();
    qeval(&mut p, "[1,2,3] = [_,_,_]");
    qnull(&mut p, "[1,2,3] = [__,__,__]");
}

#[test]
fn test_singleton_vars() {
    let pol = "f(x,y,z) if y = z;";
    let err = polar().load_str(pol).unwrap_err();
    assert!(err.context.is_some());
    assert!(matches!(
        err.kind,
        ErrorKind::Validation(ValidationError::SingletonVariable { .. })
    ))
}

#[test]
fn test_unknown_specializer_warning() -> TestResult {
    let p = polar();
    p.load_str("f(_: A);")?;
    let out = p.next_message().unwrap();
    assert!(matches!(&out.kind, MessageKind::Warning));
    assert_eq!(
        &out.msg,
        "Unknown specializer A\n001: f(_: A);\n          ^"
    );
    Ok(())
}

#[test]
fn test_missing_actor_hint() -> TestResult {
    let p = polar();

    p.register_constant(sym!("Organization"), term!(true))?;
    p.register_constant(sym!("User"), term!(true))?;

    let policy = r#"
resource Organization {
    roles = ["owner"];
    permissions = ["read"];

    "read" if "owner";
}

has_role(user: User, "owner", organization: Organization) if
    organization.owner_id = user.id;
"#;
    let err = p.load_str(policy).expect_err("Expected validation error");
    assert!(matches!(&err.kind, ErrorKind::Validation(_)));
    assert!(format!("{}", err)
        .contains("Perhaps you meant to add an actor block to the top of your policy, like this:"));

    Ok(())
}

#[test]
fn test_missing_resource_hint() -> TestResult {
    let p = Polar::new();

    let repo_instance = ExternalInstance {
        instance_id: 1,
        constructor: None,
        repr: None,
    };
    let repo_term = term!(Value::ExternalInstance(repo_instance.clone()));
    let repo_name = sym!("Repository");
    p.register_constant(repo_name.clone(), repo_term)?;
    p.register_mro(repo_name, vec![repo_instance.instance_id])?;

    let organization_instance = ExternalInstance {
        instance_id: 2,
        constructor: None,
        repr: None,
    };
    let organization_term = term!(Value::ExternalInstance(organization_instance.clone()));
    let organization_name = sym!("Organization");
    p.register_constant(organization_name.clone(), organization_term)?;
    p.register_mro(organization_name, vec![organization_instance.instance_id])?;

    let user_instance = ExternalInstance {
        instance_id: 3,
        constructor: None,
        repr: None,
    };
    let user_term = term!(Value::ExternalInstance(user_instance.clone()));
    let user_name = sym!("User");
    p.register_constant(user_name.clone(), user_term)?;
    p.register_mro(user_name, vec![user_instance.instance_id])?;

    let policy = r#"
actor User {}
resource Organization {
    roles = ["owner"];
    permissions = ["read"];

    "read" if "owner";
}

has_role(user: User, "owner", organization: Organization) if
    organization.owner_id = user.id;

has_role(user: User, "owner", repository: Repository) if
    repository.owner_id = user.id;
"#;
    let err = p.load_str(policy).expect_err("Expected validation error");
    assert!(matches!(
        &err.kind,
        ErrorKind::Validation(ValidationError::InvalidRule { .. })
    ));
    assert!(err
        .to_string()
        .contains("Perhaps you meant to add a resource block to your policy, like this:"));

    Ok(())
}

#[test]
fn test_and_or_warning() -> TestResult {
    let p = polar();

    // free-standing OR is fine
    p.load_str("f(x) if x > 1 or x < 3;")?;

    p.clear_rules();

    // OR with explicit parenthesis is fine (old behaviour)
    p.load_str("f(x) if x = 1 and (x > 1 or x < 3);")?;

    p.clear_rules();

    // OR with parenthesized AND is fine (new default)
    p.load_str("f(x) if (x = 1 and x > 1) or x < 3;")?;

    p.clear_rules();

    // Add whitespace to make sure it can find parentheses wherever they are
    p.load_str("f(x) if (\n\t    x = 1 and  x > 1) or x < 3;")?;

    p.clear_rules();
    p.load_str("f(x) if x = 1 and x > 1 or x < 3;")?;
    let mut messages = vec![];
    while let Some(msg) = p.next_message() {
        messages.push(msg);
    }
    assert!(messages.iter().any(|msg| {
      matches!(&msg.kind, MessageKind::Warning) &&
      (msg.msg ==
        "Expression without parentheses could be ambiguous. \nPrior to 0.20, `x and y or z` would parse as `x and (y or z)`. \nAs of 0.20, it parses as `(x and y) or z`, matching other languages. \n\n\n001: f(x) if x = 1 and x > 1 or x < 3;\n             ^")
    }));

    p.clear_rules();
    p.load_str("f(x) if x = 1 or x > 1 and x < 3;")?;

    let mut msgs: Vec<Message> = vec![];
    while let Some(msg) = p.next_message() {
        msgs.push(msg);
    }
    assert!(msgs.iter().any(|msg| {
      matches!(&msg.kind, MessageKind::Warning) &&
      (msg.msg ==
        "Expression without parentheses could be ambiguous. \nPrior to 0.20, `x and y or z` would parse as `x and (y or z)`. \nAs of 0.20, it parses as `(x and y) or z`, matching other languages. \n\n\n001: f(x) if x = 1 or x > 1 and x < 3;\n                      ^")
    }));

    Ok(())
}

#[test]
fn test_print() -> TestResult {
    // TODO: If POLAR_LOG is on this test will fail.
    let p = polar();
    p.load_str("f(x,y,z) if print(x, y, z);")?;
    let mut messages = vec![];
    let message_handler = |output: &Message| {
        messages.push(output.clone());
    };
    let q = p.new_query("f(1, 2, 3)", false)?;
    let _results = query_results!(q, @msgs message_handler);
    assert!(messages
        .iter()
        .any(|msg| { matches!(&msg.kind, MessageKind::Print) && (msg.msg == "1, 2, 3") }));
    Ok(())
}

#[test]
fn test_unknown_specializer_suggestions() -> TestResult {
    let p = polar();
    p.load_str("f(s: string) if s;")?;
    let msg = p.next_message().unwrap();
    assert!(matches!(&msg.kind, MessageKind::Warning));
    assert_eq!(
        &msg.msg,
        "Unknown specializer string, did you mean String?\n001: f(s: string) if s;\n          ^"
    );
    Ok(())
}

#[test]
fn test_partial_grounding() -> TestResult {
    let rules = r#"
        f(x, n) if n > 0 and x.n = n;
        g(x, n) if x.n = n and n > 0;"#;
    let mut p = polar();
    p.load_str(rules)?;

    qvar(&mut p, "f({n:1},x)", "x", vec![value!(1)]);
    qvar(&mut p, "g({n:1},x)", "x", vec![value!(1)]);
    qnull(&mut p, "f({n:1},x) and x = 2");
    qnull(&mut p, "g({n:1},x) and x = 2");
    Ok(())
}

#[test]
fn test_dict_destructuring() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"
        foo(x, _: {x});
        goo(x, y) if y matches {x};
        moo(x, {x});
        roo(a, {a, b: a});
        too(a, _: {a, b: a});
   "#,
    )?;
    for s in &["foo", "goo"] {
        qeval(&mut p, &format!("{}(1, {{x: 1}})", s));
        qnull(&mut p, &format!("{}(2, {{x: 1}})", s));
        qnull(&mut p, &format!("{}(1, {{x: 2}})", s));
        qeval(&mut p, &format!("{}(2, {{x: 2, y: 3}})", s));
    }

    qeval(&mut p, "moo(1, {x: 1})");
    qnull(&mut p, "moo(2, {x: 1})");
    qnull(&mut p, "moo(1, {x: 2})");
    qnull(&mut p, "moo(2, {x: 2, y: 3})");

    qeval(&mut p, "roo(1, {a: 1, b: 1})");
    qeval(&mut p, "too(1, {a: 1, b: 1})");
    qnull(&mut p, "roo(1, {a: 1, b: 1, c: 2})");
    qeval(&mut p, "too(1, {a: 1, b: 1, c: 2})");
    Ok(())
}

#[ignore]
#[test]
fn test_dict_destructuring_broken() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"
        too(a, _: {a, b: a});
   "#,
    )?;
    // currently hits an unimplemented code path in vm.rs
    qeval(&mut p, "a={a} and too(a, {a, b: a})");
    Ok(())
}

#[test]
fn test_rest_vars() -> TestResult {
    let mut p = polar();
    qvar(&mut p, "[1,2,3] = [*rest]", "rest", vec![value!([1, 2, 3])]);
    qvar(&mut p, "[1,2,3] = [1,*rest]", "rest", vec![value!([2, 3])]);
    qvar(&mut p, "[1,2,3] = [1,2,*rest]", "rest", vec![value!([3])]);
    qvar(&mut p, "[1,2,3] = [1,2,3,*rest]", "rest", vec![value!([])]);
    qnull(&mut p, "[1,2,3] = [1,2,3,4,*_rest]");

    p.load_str(
        r#"member(x, [x, *_rest]);
           member(x, [_first, *rest]) if member(x, rest);"#,
    )?;
    qeval(&mut p, "member(1, [1,2,3])");
    qeval(&mut p, "member(3, [1,2,3])");
    qeval(&mut p, "not member(4, [1,2,3])");
    qvar(&mut p, "member(x, [1,2,3])", "x", values![1, 2, 3]);

    p.clear_rules();

    p.load_str(
        r#"append([], x, x);
           append([first, *rest], x, [first, *tail]) if append(rest, x, tail);"#,
    )?;
    qeval(&mut p, "append([], [], [])");
    qeval(&mut p, "append([], [1,2,3], [1,2,3])");
    qeval(&mut p, "append([1], [2,3], [1,2,3])");
    qeval(&mut p, "append([1,2], [3], [1,2,3])");
    qeval(&mut p, "append([1,2,3], [], [1,2,3])");
    qeval(&mut p, "not append([1,2,3], [4], [1,2,3])");

    qeval(
        &mut p,
        "a = [1, *b] and b = [2, *c] and c=[3] and 1 in a and 2 in a and 3 in a",
    );

    let a = &var(&mut p, "[*c] in [*a] and [*b] in [*d] and b = 1", "a")[0];
    // check that a isn't bound to [b]
    assert!(!matches!(a, Value::List(b) if matches!(b[0].value(), Value::Number(_))));
    Ok(())
}

#[test]
fn test_circular_data() -> TestResult {
    let mut p = polar();
    qeval(&mut p, "x = [x] and x in x");
    qeval(&mut p, "y = {y:y} and [\"y\", y] in y");
    qruntime!(
        "x = [x, y] and y = [y, x] and x = y",
        RuntimeError::StackOverflow { .. }
    );
    Ok(())
}

#[test]
fn test_data_filtering_dict_specializers() -> TestResult {
    let pol_a = "allow(x, \"read\", _y: { x: x });";
    let pol_b = "allow(x, \"read\", _y) if x = _y.x;";
    let query = "allow(\"gwen\", \"read\", x)";
    let p = polar();
    p.load_str(pol_a)?;
    let mut res_a = query_results!(p.new_query(query, false)?);
    p.clear_rules();
    p.load_str(pol_b)?;
    let mut res_b = query_results!(p.new_query(query, false)?);
    assert_eq!(res_a.len(), 1);
    assert_eq!(res_b.len(), 1);
    let key = sym!("x");
    let res_a = res_a[0].0.remove(&key).unwrap();
    let res_b = res_b[0].0.remove(&key).unwrap();
    assert!(commute_ops(&res_a, &res_b));
    Ok(())
}

#[test]
fn test_data_filtering_pattern_specializers() -> TestResult {
    let pol_a = "allow(x, \"read\", _y: Dictionary{ x: x });";
    let pol_b = "allow(x, \"read\", _y: Dictionary) if x = _y.x;";
    let query = "allow(\"gwen\", \"read\", x)";
    let p = polar();
    p.load_str(pol_a)?;
    let mut res_a = query_results!(p.new_query(query, false)?);
    p.clear_rules();
    p.load_str(pol_b)?;
    let mut res_b = query_results!(p.new_query(query, false)?);
    assert_eq!(res_a.len(), 1);
    assert_eq!(res_b.len(), 1);
    let key = sym!("x");
    let res_a = res_a[0].0.remove(&key).unwrap();
    let res_b = res_b[0].0.remove(&key).unwrap();
    assert!(commute_ops(&res_a, &res_b));
    Ok(())
}

#[test]
fn test_in_op() -> TestResult {
    let mut p = polar();
    p.load_str("f(x, y) if x in y;")?;
    qeval(&mut p, "f(1, [1,2,3])");
    qvar(&mut p, "f(x, [1,2,3])", "x", values![1, 2, 3]);

    // Failure.
    qnull(&mut p, "4 in [1,2,3]");
    qeval(&mut p, "4 in [1,2,3] or 1 in [1,2,3]");

    // Make sure we scan the whole list.
    let q = p.new_query("1 in [1, 2, x, 1]", false)?;
    let results = query_results!(q);
    assert_eq!(results.len(), 3);
    assert!(results[0].0.is_empty());
    assert_eq!(
        results[1].0.get(&Symbol("x".to_string())).unwrap().clone(),
        value!(1)
    );
    assert!(results[2].0.is_empty());

    // This returns 3 results, with 1 binding each.
    let q = p.new_query("f(1, [x,y,z])", false)?;
    let results = query_results!(q);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0[&sym!("x")], value!(1));
    assert_eq!(results[1].0[&sym!("y")], value!(1));
    assert_eq!(results[2].0[&sym!("z")], value!(1));

    qeval(&mut p, "f({a:1}, [{a:1}, b, c])");

    // Negation.
    qeval(&mut p, "not (4 in [1,2,3])");
    qnull(&mut p, "not (1 in [1,2,3])");
    qnull(&mut p, "not (2 in [1,2,3])");
    qnull(&mut p, "not (3 in [1,2,3])");

    // Nothing is in an empty list.
    qnull(&mut p, "x in []");
    qnull(&mut p, "1 in []");
    qnull(&mut p, "\"foo\" in []");
    qnull(&mut p, "[] in []");
    qeval(&mut p, "not x in []");
    qeval(&mut p, "not 1 in []");
    qeval(&mut p, "not \"foo\" in []");
    qeval(&mut p, "not [] in []");

    // test on rest variables
    qeval(
        &mut p,
        "a = [1, *b] and b = [2, *c] and c = [3] and 1 in a and 2 in a and 3 in a",
    );
    Ok(())
}

#[test]
fn test_head_patterns() -> TestResult {
    let p = polar();
    p.load_str("f(x: Integer, _: Dictionary{x:x});")?;
    let results = query_results!(p.new_query("f(x, {x:9})", false)?);
    assert_eq!(results[0].0[&sym!("x")], value!(9));
    Ok(())
}

#[test]
fn test_matches() {
    let mut p = polar();
    qnull(&mut p, "1 matches 2");
    qeval(&mut p, "1 matches 1");
    // This doesn't fail because `y` is parsed as an unknown specializer
    // qnull(&mut p, "x = 1 and y = 2 and x matches y");
    qeval(&mut p, "x = {foo: 1} and x matches {foo: 1}");
    qeval(&mut p, "x = {foo: 1, bar: 2} and x matches {foo: 1}");
    qnull(&mut p, "x = {foo: 1} and x matches {foo: 1, bar: 2}");
    qnull(&mut p, "x = {foo: 1} and x matches {foo: 2}");
    qeval(&mut p, "x matches Integer and x = 1");
}

#[test]
fn test_keyword_call() {
    qparse!("cut(a) if a;", ParseError::ReservedWord { .. });
    qparse!("debug(a) if a;", ParseError::ReservedWord { .. });
    qparse!(
        "foo(debug) if debug = 1;",
        ParseError::UnrecognizedToken { .. }
    );
}

#[test]
fn test_keyword_dot() -> TestResult {
    // field accesses of reserved words are allowed
    let mut p = polar();
    p.load_str(
        r#"
        f(a, b) if a.in(b);
        g(a, b) if a.new(b);
    "#,
    )?;
    qeval(
        &mut p,
        "x = {debug: 1, new: 2, type: 3} and x.debug + x.new = x.type",
    );
    Ok(())
}

/// Test that rule heads work correctly when unification or specializers are used.
#[test]
fn test_unify_rule_head() -> TestResult {
    qparse!("f(Foo{a: 1});", ParseError::UnrecognizedToken { .. });
    qparse!(
        "f(new Foo(a: Foo{a: 1}));",
        ParseError::UnrecognizedToken { .. }
    );
    qparse!("f(x: new Foo(a: 1));", ParseError::ReservedWord { .. });
    qparse!(
        "f(x: Foo{a: new Foo(a: 1)});",
        ParseError::ReservedWord { .. }
    );

    let p = polar();
    p.register_constant(sym!("Foo"), term!(true))?;
    p.load_str(
        r#"f(_: Foo{a: 1}, x) if x = 1;
           g(_: Foo{a: Foo{a: 1}}, x) if x = 1;"#,
    )?;

    let q = p.new_query("f(new Foo(a: 1), x)", false)?;
    let (results, _externals) = query_results_with_externals(q);
    assert_eq!(results[0].0[&sym!("x")], value!(1));

    let q = p.new_query("g(new Foo(a: new Foo(a: 1)), x)", false)?;
    let (results, _externals) = query_results_with_externals(q);
    assert_eq!(results[0].0[&sym!("x")], value!(1));
    Ok(())
}

/// Test that cut commits to all choice points before the cut, not just the last.
#[test]
fn test_cut() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"a(x) if x = 1 or x = 2;
           b(x) if x = 3 or x = 4;
           bcut(x) if (x = 3 or x = 4) and cut;
           c(a, b) if a(a) and b(b) and cut;
           c_no_cut(a, b) if a(a) and b(b);
           c_partial_cut(a, b) if a(a) and bcut(b);
           c_another_partial_cut(a, b) if a(a) and cut and b(b);"#,
    )?;

    // Ensure we return multiple results without a cut.
    qvars(
        &mut p,
        "c_no_cut(a, b)",
        &["a", "b"],
        values![[1, 3], [1, 4], [2, 3], [2, 4]],
    );

    // Ensure that only one result is returned when cut is at the end.
    qvars(&mut p, "c(a, b)", &["a", "b"], values![[1, 3]]);

    // Make sure that cut in `bcut` does not affect `c_partial_cut`.
    // If it did, only one result would be returned, [1, 3].
    qvars(
        &mut p,
        "c_partial_cut(a, b)",
        &["a", "b"],
        values![[1, 3], [2, 3]],
    );

    // Make sure cut only affects choice points before it.
    qvars(
        &mut p,
        "c_another_partial_cut(a, b)",
        &["a", "b"],
        values![[1, 3], [1, 4]],
    );

    p.clear_rules();

    p.load_str("f(x) if (x = 1 and cut) or x = 2;")?;
    qvar(&mut p, "f(x)", "x", values![1]);
    qeval(&mut p, "f(1)");
    qeval(&mut p, "f(2)");
    Ok(())
}

#[test]
fn test_forall() -> TestResult {
    let mut p = polar();
    p.load_str("all_ones(l) if forall(item in l, item = 1);")?;

    qnull(&mut p, "all_ones([2])");

    qeval(&mut p, "all_ones([1])");
    qeval(&mut p, "all_ones([1, 1, 1])");
    qnull(&mut p, "all_ones([1, 2, 1])");

    p.clear_rules();

    p.load_str("not_ones(l) if forall(item in l, item != 1);")?;
    qnull(&mut p, "not_ones([1])");
    qeval(&mut p, "not_ones([2, 3, 4])");

    qnull(&mut p, "forall(x = 2 or x = 3, x != 2)");
    qnull(&mut p, "forall(x = 2 or x = 3, x != 3)");
    qeval(&mut p, "forall(x = 2 or x = 3, x = 2 or x = 3)");
    qeval(&mut p, "forall(x = 1, x = 1)");
    qeval(&mut p, "forall(x in [2, 3, 4], x > 1)");

    p.clear_rules();

    p.load_str(
        r#"g(1);
           g(2);
           g(3);"#,
    )?;
    qeval(&mut p, "forall(g(x), x in [1, 2, 3])");

    p.clear_rules();

    p.load_str(
        r#"test(_: {x: 1}, y) if y = 1;
           test(_: {y: 1}, y) if y = 2;
           test(_: {z: 1}, y) if y = 3;"#,
    )?;
    qeval(
        &mut p,
        "forall(test({x: 1, y: 1, z: 1}, y), y in [1, 2, 3])",
    );
    Ok(())
}

#[test]
fn test_emoji_policy() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"
                    👩‍🔧("👩‍🦰");
                    allow(👩, "🛠", "🚙") if 👩‍🔧(👩);
                "#,
    )?;
    qeval(&mut p, r#"allow("👩‍🦰","🛠","🚙")"#);
    qnull(&mut p, r#"allow("🧟","🛠","🚙")"#);
    Ok(())
}

#[test]
/// Check that boolean expressions evaluate without requiring "= true".
fn test_boolean_expression() {
    let mut p = polar();
    qeval(&mut p, "a = {t: true, f: false} and a.t"); // Succeeds because t is true.
    qnull(&mut p, "a = {t: true, f: false} and a.f"); // Fails because `f` is not true.
    qnull(&mut p, "a = {t: true, f: false} and a.f and a.t"); // Fails because `f` is not true.
    qeval(&mut p, "a = {t: true, f: false} and (a.f or a.t)"); // Succeeds because `t` is true.

    qeval(&mut p, "true");
    qnull(&mut p, "false");
    qeval(&mut p, "a = true and a");
    qnull(&mut p, "a = false and a");
}

#[test]
fn test_float_parsing() {
    let mut p = polar();
    qvar(&mut p, "x=1+1", "x", values![2]);
    qvar(&mut p, "x=1+1.5", "x", values![2.5]);
    qvar(&mut p, "x=1.e+5", "x", values![1e5]);
    qvar(&mut p, "x=1e+5", "x", values![1e5]);
    qvar(&mut p, "x=1e5", "x", values![1e5]);
    qvar(&mut p, "x=1e-5", "x", values![1e-5]);
    qvar(&mut p, "x=1.e-5", "x", values![1e-5]);
    qvar(&mut p, "x=1.0e+15", "x", values![1e15]);
    qvar(&mut p, "x=1.0E+15", "x", values![1e15]);
    qvar(&mut p, "x=1.0e-15", "x", values![1e-15]);
}

#[test]
fn test_assignment() {
    let mut p = polar();
    qeval(&mut p, "x := 5 and x == 5");
    qruntime!(
        "x := 5 and x := 6",
        RuntimeError::TypeError { msg: s, .. },
        s == "Can only assign to unbound variables, x is not unbound."
    );
    qnull(&mut p, "x := 5 and x > 6");
    qeval(&mut p, "x := y and y = 6 and x = 6");

    // confirm old syntax -> parse error
    qparse!("f(x) := g(x);", ParseError::UnrecognizedToken { .. });
}

#[test]
fn test_rule_index() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(1, 1, "x");
           f(1, 1, "y");
           f(1, x, "y") if x = 2;
           f(1, 2, {b: "y"});
           f(1, 3, {c: "z"});"#,
    )?;
    // Exercise the index.
    qeval(&mut p, r#"f(1, 1, "x")"#);
    qeval(&mut p, r#"f(1, 1, "y")"#);
    qvar(&mut p, r#"f(1, x, "y")"#, "x", values![1, 2]);
    qnull(&mut p, r#"f(1, 1, "z")"#);
    qnull(&mut p, r#"f(1, 2, "x")"#);
    qeval(&mut p, r#"f(1, 2, {b: "y"})"#);
    qeval(&mut p, r#"f(1, 3, {c: "z"})"#);
    Ok(())
}

#[test]
fn test_fib() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"fib(0, 1) if cut;
           fib(1, 1) if cut;
           fib(n, a+b) if fib(n-1, a) and fib(n-2, b);"#,
    )?;
    qvar(&mut p, r#"fib(0, x)"#, "x", values![1]);
    qvar(&mut p, r#"fib(1, x)"#, "x", values![1]);
    qvar(&mut p, r#"fib(2, x)"#, "x", values![2]);
    qvar(&mut p, r#"fib(3, x)"#, "x", values![3]);
    qvar(&mut p, r#"fib(4, x)"#, "x", values![5]);
    qvar(&mut p, r#"fib(5, x)"#, "x", values![8]);
    Ok(())
}

#[test]
fn test_duplicated_rule() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"f(1);
           f(1);"#,
    )?;
    qvar(&mut p, "f(x)", "x", values![1, 1]);
    Ok(())
}

#[test]
fn test_numeric_applicability() -> TestResult {
    let mut p = polar();
    let eps = f64::EPSILON;
    let nan1 = f64::NAN;
    let nan2 = f64::from_bits(f64::NAN.to_bits() | 1);
    assert!(eps.is_normal() && nan1.is_nan() && nan2.is_nan());
    p.register_constant(sym!("eps"), term!(eps))?;
    p.register_constant(sym!("nan1"), term!(nan1))?;
    p.register_constant(sym!("nan2"), term!(nan2))?;
    p.load_str(
        r#"f(0);
           f(1);
           f(9007199254740991); # (1 << 53) - 1
           f(9007199254740992); # (1 << 53)
           f(9223372036854775807); # i64::MAX
           f(-9223372036854775807); # i64::MIN + 1
           f(9223372036854776000.0); # i64::MAX as f64
           f(nan1); # NaN"#,
    )?;
    qeval(&mut p, "f(0)");
    qeval(&mut p, "f(0.0)");
    qnull(&mut p, "f(eps)");
    qeval(&mut p, "f(1)");
    qeval(&mut p, "f(1.0)");
    qnull(&mut p, "f(1.0000000000000002)");
    qnull(&mut p, "f(9007199254740990)");
    qnull(&mut p, "f(9007199254740990.0)");
    qeval(&mut p, "f(9007199254740991)");
    qeval(&mut p, "f(9007199254740991.0)");
    qeval(&mut p, "f(9007199254740992)");
    qeval(&mut p, "f(9007199254740992.0)");
    qeval(&mut p, "f(9223372036854775807)");
    qeval(&mut p, "f(-9223372036854775807)");
    qeval(&mut p, "f(9223372036854776000.0)");
    qeval(&mut p, "f(nan1)");
    qnull(&mut p, "f(nan2)");
    Ok(())
}

#[test]
fn test_external_unify() -> TestResult {
    let p = polar();
    p.load_str(
        r#"selfEq(x) if eq(x, x);
           eq(x, x);"#,
    )?;

    let q = p.new_query("selfEq(new Foo())", false)?;
    let (results, _externals) = query_results_with_externals(q);
    assert_eq!(results.len(), 1);

    let q = p.new_query("eq(new Foo(), new Foo())", false)?;
    let (results, _externals) = query_results_with_externals(q);
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn test_list_results() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"delete([x, *xs], x, ys) if delete(xs, x, ys);
           delete([x, *xs], z, [x, *ys]) if
               x != z and delete(xs, z, ys);
           delete([], _, []);"#,
    )?;
    qeval(&mut p, "delete([1,2,3,2,1],2,[1,3,1])");
    qvar(
        &mut p,
        "delete([1,2,3,2,1],2,result)",
        "result",
        vec![value!([1, 3, 1])],
    );

    qvar(&mut p, "[1,2] = [1,*ys]", "ys", vec![value!([2])]);
    qvar(
        &mut p,
        "[1,2,*xs] = [1,*ys] and [1,2,3] = [1,*ys]",
        "xs",
        vec![value!([3])],
    );
    qvar(
        &mut p,
        "[1,2,*xs] = [1,*ys] and [1,2,3] = [1,*ys]",
        "ys",
        vec![value!([2, 3])],
    );
    qvar(
        &mut p,
        "[1,2,*xs] = [1,*ys] and [1,2,3] = [1,*ys]",
        "ys",
        vec![value!([2, 3])],
    );
    qeval(&mut p, "xs = [2] and [1,2] = [1, *xs]");
    qnull(&mut p, "[1, 2] = [2, *ys]");
    Ok(())
}

#[test]
fn test_expressions_in_lists() -> TestResult {
    let mut p = polar();
    p.load_str(
        r#"scope(actor: Dictionary, "read", "Person", filters) if
               filters = ["id", "=", actor.id];"#,
    )?;
    qeval(
        &mut p,
        r#"scope({id: 1}, "read", "Person", ["id", "=", 1])"#,
    );
    qnull(
        &mut p,
        r#"scope({id: 2}, "read", "Person", ["id", "=", 1])"#,
    );
    qnull(
        &mut p,
        r#"scope({id: 1}, "read", "Person", ["not_id", "=", 1])"#,
    );
    qeval(&mut p, r#"d = {x: 1} and [d.x, 1+1] = [1, 2]"#);
    qvar(
        &mut p,
        r#"d = {x: 1} and [d.x, 1+1] = [1, *rest]"#,
        "rest",
        vec![value!([2])],
    );
    Ok(())
}

#[test]
fn test_list_matches() {
    let mut p = polar();
    qeval(&mut p, "[] matches []");
    qnull(&mut p, "[1] matches []");
    qnull(&mut p, "[] matches [1]");
    qnull(&mut p, "[1, 2] matches [1, 2, 3]");
    qnull(&mut p, "[2, 1] matches [1, 2]");
    qeval(&mut p, "[1, 2, 3] matches [1, 2, 3]");
    qnull(&mut p, "[1, 2, 3] matches [1, 2]");

    qnull(&mut p, "[x] matches []");
    qnull(&mut p, "[] matches [x]");
    qnull(&mut p, "[1, 2, x] matches [1, 2]");
    qnull(&mut p, "[1, x] matches [1, 2, 3]");
    qnull(&mut p, "[2, x] matches [1, 2]");
    qvar(&mut p, "[1, 2, x] matches [1, 2, 3]", "x", values![3]);
    qnull(&mut p, "[1, 2, 3] matches [1, x]");

    qvar(&mut p, "[] matches [*ys]", "ys", vec![value!([])]);
    qvar(&mut p, "[*xs] matches []", "xs", vec![value!([])]);
    qvar(&mut p, "[*xs] matches [1]", "xs", vec![value!([1])]);
    qvar(&mut p, "[1] matches [*ys]", "ys", vec![value!([1])]);
    qeval(&mut p, "[xs] matches [*ys]");
    qeval(&mut p, "[*xs] matches [ys]");
    qeval(&mut p, "[*xs] matches [*ys]");
    qvar(&mut p, "[1,2,3] matches [1,2,*xs]", "xs", vec![value!([3])]);
    qvar(
        &mut p,
        "[1,2,*xs] matches [1,2,3,*ys]",
        "xs",
        vec![value!([3, Value::RestVariable(Symbol::new("ys"))])],
    );
}

#[test]
fn test_builtin_iterables() {
    let mut p = polar();

    qnull(&mut p, r#"x in """#);
    qvar(
        &mut p,
        "x in \"abc\"",
        "x",
        vec![value!("a"), value!("b"), value!("c")],
    );
    qnull(&mut p, "x in {}");
    qvar(
        &mut p,
        "x in {a: 1, b: 2}",
        "x",
        vec![value!(["a", 1]), value!(["b", 2])],
    );
    qeval(&mut p, r#"["a", 1] in {a: 1, b: 2}"#);
    qvar(
        &mut p,
        "[x, _] in {a: 1, b: 2}",
        "x",
        vec![value!("a"), value!("b")],
    );
    qeval(&mut p, r#"["a", 1] in {a: 1, b: 2}"#);
    qvar(
        &mut p,
        "[_, x] in {a: 1, b: 2}",
        "x",
        vec![value!(1), value!(2)],
    );

    qeval(&mut p, r#""b" in "abc""#);
    qnull(&mut p, r#""d" in "abc""#);
    qeval(&mut p, r#"forall(x in "abc", x in "abacus")"#);
    qnull(&mut p, r#"forall(x in "abcd", x in "abacus")"#);
}

#[test]
/// Regression test for lookups done in rule head: old behavior was for query to succeed
/// despite argument not matching lookup result
fn test_lookup_in_rule_head() -> TestResult {
    let p = polar();
    p.register_constant(sym!("Foo"), term!(true))?;
    p.load_str(r#"test(foo: Foo, foo.bar());"#)?;

    let good_q = p.new_query("test(new Foo(), 1)", false)?;

    let mock_foo_lookup =
        |_, _, _, _: Option<Vec<Term>>, _: Option<BTreeMap<Symbol, Term>>| Some(term!(1));
    let results = query_results!(good_q, mock_foo_lookup);
    assert_eq!(results.len(), 1);

    let bad_q = p.new_query("test(new Foo(), 2)", false)?;
    let results = query_results!(bad_q, mock_foo_lookup);
    assert_eq!(results.len(), 0);
    Ok(())
}

#[test]
fn test_default_rule_types() -> TestResult {
    let p = polar();

    // This should fail
    let e = p
        .load_str(r#"has_permission("leina", "eat", "food");"#)
        .expect_err("Expected validation error");
    assert!(matches!(e.kind, ErrorKind::Validation(_)));
    let e = p
        .load_str(r#"allow("leina", "food");"#)
        .expect_err("Expected validation error");
    assert!(matches!(e.kind, ErrorKind::Validation(_)));
    let e = p
        .load_str(r#"allow_field("leina", "food");"#)
        .expect_err("Expected validation error");
    assert!(matches!(e.kind, ErrorKind::Validation(_)));
    let e = p
        .load_str(r#"allow_request("leina", "eat", "food");"#)
        .expect_err("Expected validation error");
    assert!(matches!(e.kind, ErrorKind::Validation(_)));

    // This should succeed
    // TODO: should we emit warnings if rules with union specializers are loaded
    // but no union types have been declared?
    p.load_str(
        r#"
    has_permission(_actor: Actor, "eat", _resource: Resource);
    has_permission(_actor: Actor, "eat", _resource: Actor);
    has_role(_actor: Actor, "member", _resource: Resource);
    has_role(_actor: Actor, "member", _resource: Actor);
    has_relation(_actor: Actor, "any", _other: Actor);
    has_relation(_actor: Resource, "any", _other: Actor);
    has_relation(_actor: Resource, "any", _other: Resource);
    has_relation(_actor: Actor, "any", _other: Resource);
    allow("a", "b", "c");
    allow_field("a", "b", "c", "d");
    allow_request("a", "b");
    "#,
    )?;
    // Make sure there are no warnings
    assert!(p.next_message().is_none());
    Ok(())
}

#[test]
fn test_suggested_rule_specializer() -> TestResult {
    let p = polar();

    let repo_instance = ExternalInstance {
        instance_id: 1,
        constructor: None,
        repr: None,
    };
    let repo_term = term!(Value::ExternalInstance(repo_instance.clone()));
    let repo_name = sym!("Repository");
    p.register_constant(repo_name.clone(), repo_term)?;
    p.register_mro(repo_name, vec![repo_instance.instance_id])?;

    let user_instance = ExternalInstance {
        instance_id: 2,
        constructor: None,
        repr: None,
    };
    let user_term = term!(Value::ExternalInstance(user_instance.clone()));
    let user_name = sym!("User");
    p.register_constant(user_name.clone(), user_term)?;
    p.register_mro(user_name, vec![user_instance.instance_id])?;

    let policy = r#"
actor User {}
resource Repository {
    permissions = ["read"];
    roles = ["contributor"];

    "read" if "contributor";
}

has_role(actor: User, role_name, repository: Repository) if
    role in actor.roles and
    role_name = role.name and
    repository = role.repository;
"#;

    let err = p.load_str(policy).expect_err("Expected validation error");
    assert!(matches!(&err.kind, ErrorKind::Validation(_)));
    assert!(format!("{}", err).contains(
        "Failed to match because: Parameter `role_name` expects a String type constraint."
    ));

    Ok(())
}

// If you declare a relation & a shorthand rule that references the relationship but don't
// implement a corresponding has_relation linking the two resources, you'll see a
// `MissingRequiredRule` error.
#[test]
fn test_missing_required_rule_type() -> TestResult {
    let p = Polar::new();

    let repo_instance = ExternalInstance {
        instance_id: 1,
        constructor: None,
        repr: None,
    };
    let repo_term = term!(Value::ExternalInstance(repo_instance.clone()));
    let repo_name = sym!("Repository");
    p.register_constant(repo_name.clone(), repo_term)?;
    p.register_mro(repo_name, vec![repo_instance.instance_id])?;

    let issue_instance = ExternalInstance {
        instance_id: 2,
        constructor: None,
        repr: None,
    };
    let issue_term = term!(Value::ExternalInstance(issue_instance.clone()));
    let issue_name = sym!("Issue");
    p.register_constant(issue_name.clone(), issue_term)?;
    p.register_mro(issue_name, vec![issue_instance.instance_id])?;

    let user_instance = ExternalInstance {
        instance_id: 3,
        constructor: None,
        repr: None,
    };
    let user_term = term!(Value::ExternalInstance(user_instance.clone()));
    let user_name = sym!("User");
    p.register_constant(user_name.clone(), user_term)?;
    p.register_mro(user_name, vec![user_instance.instance_id])?;

    let policy = r#"
actor User {}
resource Repository {
    relations = {owner: User};
}

resource Issue {
    roles = ["write"];
    relations = {repo: Repository};
    "write" if "owner" on "repo";
}

allow(actor, action, resource) if has_permission(actor, action, resource);
"#;

    let err = p.load_str(policy).expect_err("Expected validation error");
    assert!(matches!(
        &err.kind,
        ErrorKind::Validation(ValidationError::MissingRequiredRule { .. })
    ));
    assert!(err
        .to_string()
        .contains("Missing implementation for required rule has_relation("));
    Ok(())
}
