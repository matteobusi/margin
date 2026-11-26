//! Pipeline test: checker (with the UNSOUND test kernel) -> journal -> render.

use margin_canon::*;
use margin_checker::{run, ProverInput, UncheckedKernel};

#[test]
fn checker_journal_renders_statement_but_not_proof() {
    let mut a = Arena::new();
    let prop = a.sort(Level::Zero);
    let true_c = a.cnst("True", vec![]);
    let env = [Decl::Inductive(InductiveBlock {
        types: vec![IndType {
            name: Name::from_dotted("True"),
            lparams: vec![],
            ty: prop,
            all: vec![Name::from_dotted("True")],
            ctors: vec![Name::from_dotted("True.intro")],
            is_rec: false,
            is_reflexive: false,
            is_unsafe: false,
            num_params: 0,
            num_indices: 0,
            num_nested: 0,
        }],
        ctors: vec![CtorVal {
            name: Name::from_dotted("True.intro"),
            lparams: vec![],
            ty: true_c,
            is_unsafe: false,
            induct: Name::from_dotted("True"),
            cidx: 0,
            num_params: 0,
            num_fields: 0,
        }],
        recs: vec![],
    })];
    let intro = a.cnst("True.intro", vec![]);
    let secret = Decl::Theorem { name: Name::from_dotted("secret_lemma"), lparams: vec![], ty: true_c, val: intro };
    let lemma = a.cnst("secret_lemma", vec![]);
    let main = Decl::Theorem { name: Name::from_dotted("main"), lparams: vec![], ty: true_c, val: lemma };

    let env_set = margin_checker::Env::build(&a, env.to_vec()).expect("valid environment");
    let all: Vec<usize> = (0..env.len()).collect();
    let (env_decls, env_proof) = env_set.open(&a, &all, &[]);
    let input = ProverInput {
        env_root: env_set.tree.root,
        owner: [3; 32],
        claimed: vec![Name::from_dotted("main")],
        env_decls,
        env_proof,
        user_decls: vec![encode_decl(&a, &secret), encode_decl(&a, &main)],
    };
    // The guest reads the encoded input, so test the round trip too.
    let input = ProverInput::decode(&input.encode()).unwrap();

    let journal = run(&input, &mut UncheckedKernel).unwrap();
    let journal = margin_journal::Journal::decode(&journal.encode()).unwrap();
    let text = margin_verify::render_journal(&journal).unwrap();
    assert!(text.contains("theorem main : True"), "{text}");
    assert!(!text.contains("secret_lemma"), "{text}");
}
