//! Tests on real lean4export files shipped with nanoda (`third_party/nanoda_lib/test_resources`).

use margin_canon::lean4export::read_ndjson;
use margin_canon::{decode_decl, encode_decl, Arena, Decl};
use margin_checker::Kernel;
use margin_kernel_nanoda::NanodaKernel;

fn load(name: &str) -> (Arena, Vec<Decl>) {
    let path = format!("{}/../../third_party/nanoda_lib/test_resources/{name}/export", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    let mut arena = Arena::new();
    let decls = read_ndjson(&text, &mut arena).unwrap();
    (arena, decls)
}

#[test]
fn ce_roundtrip_of_real_export() {
    let (arena, decls) = load("ProjFromProp");
    assert_eq!(decls.len(), 4, "2 inductive blocks + 2 definitions");
    for d in &decls {
        let bytes = encode_decl(&arena, d);
        let mut fresh = Arena::new();
        let d2 = decode_decl(&bytes, &mut fresh).unwrap();
        assert_eq!(encode_decl(&fresh, &d2), bytes);
    }
}

#[test]
fn accepts_valid_inductives() {
    // PUnit and `Wrap : Prop` (one field) are well-formed inductive types.
    let (arena, decls) = load("ProjFromProp");
    NanodaKernel::default().check(&arena, &[], &decls[..2]).unwrap();
}

#[test]
fn accepts_valid_inductive_from_trusted_env() {
    let (arena, decls) = load("ProjFromProp");
    NanodaKernel::default().check(&arena, &decls[..1], &decls[1..2]).unwrap();
}

#[test]
fn rejects_projection_out_of_prop() {
    // The definitions project data out of a proof: the kernel must reject them.
    let (arena, decls) = load("ProjFromProp");
    let err = NanodaKernel::default().check(&arena, &decls[..2], &decls[2..]).unwrap_err();
    assert!(err.0.contains("infer_proj prop"), "{}", err.0);
}

#[test]
fn rejects_missing_dependency() {
    // Without the inductive types in scope, the definitions cannot check.
    let (arena, decls) = load("ProjFromProp");
    assert!(NanodaKernel::default().check(&arena, &[], &decls[2..3]).is_err());
}

#[test]
fn rejects_tampered_recursor_rule() {
    // The kernel re-derives recursors. A user block with a wrong rule must fail.
    let (mut arena, decls) = load("ProjFromProp");
    let mut wrap = decls[1].clone();
    let Decl::Inductive(b) = &mut wrap else { panic!("expected inductive") };
    let bogus = arena.sort(margin_canon::Level::Zero);
    b.recs[0].rules[0].rhs = bogus;
    assert!(NanodaKernel::default().check(&arena, &decls[..1], &[wrap]).is_err());
}

mod pipeline {
    use super::*;
    use margin_canon::Name;
    use margin_checker::{run, CheckError, ProverInput};

    /// env = {PUnit, Wrap}; `Wrap : Prop`, `Wrap.mk : Prop → Wrap`.
    fn input(user: impl FnOnce(&mut Arena) -> Vec<Decl>, claim: &str) -> ProverInput {
        let (mut arena, decls) = load("ProjFromProp");
        let env = &decls[..2];
        let user = user(&mut arena);
        let env_set = margin_checker::Env::build(&arena, env.to_vec()).expect("valid environment");
        let all: Vec<usize> = (0..env.len()).collect();
        let (env_decls, env_proof) = env_set.open(&arena, &all, &[]);
        ProverInput {
            env_root: env_set.tree.root,
            owner: [7; 32],
            self_id: [0; 32],
            module: Name::from_dotted("Test"),
            claimed: vec![Name::from_dotted(claim)],
            env_decls,
        env_proof,
            deps: vec![],
            user_decls: user.iter().map(|d| encode_decl(&arena, d)).collect(),
        }
    }

    fn thm(name: &str, ty: margin_canon::ExprId, val: margin_canon::ExprId) -> Decl {
        Decl::Theorem { name: Name::from_dotted(name), lparams: vec![], ty, val }
    }

    #[test]
    fn proves_real_theorem_and_hides_lemma() {
        let inp = input(
            |a| {
                let wrap = a.cnst("Wrap", vec![]);
                let mk = a.cnst("Wrap.mk", vec![]);
                let mk_wrap = a.app(mk, wrap);
                let secret = thm("secret", wrap, mk_wrap);
                let s = a.cnst("secret", vec![]);
                let main = thm("main", wrap, s);
                vec![secret, main]
            },
            "main",
        );
        let j = run(&inp, &mut NanodaKernel::default()).unwrap().journal().unwrap();
        assert_eq!(j.checker, margin_kernel_nanoda::NAME);
        assert_eq!(j.claims.len(), 1);
        assert!(!j.encode().windows(6).any(|w| w == b"secret"));
    }

    #[test]
    fn rejects_ill_typed_theorem() {
        // `Wrap : Prop`, so `Wrap` is not a proof of `Wrap`.
        let inp = input(
            |a| {
                let wrap = a.cnst("Wrap", vec![]);
                vec![thm("bad", wrap, wrap)]
            },
            "bad",
        );
        assert!(matches!(run(&inp, &mut NanodaKernel::default()), Err(CheckError::Kernel { .. })));
    }
}

#[test]
fn json_text_path_matches_direct_records() {
    // The JSON writer and the direct record feed must describe the same environment:
    // nanoda parses the JSON text with its normal file parser and checks it.
    use nanoda_lib::util::Config;
    let (arena, decls) = load("ProjFromProp");
    let refs: Vec<&Decl> = decls.iter().collect();
    let text = margin_canon::lean4export::write_ndjson(&arena, &refs).unwrap();
    let config: Config = serde_json::from_str(
        r#"{"use_stdin": true, "unsafe_permit_all_axioms": true, "unpermitted_axiom_hard_error": false}"#,
    )
    .unwrap();
    let (export, skipped) = nanoda_lib::parser::parse_export_file(text.as_bytes(), config).unwrap();
    assert!(skipped.is_empty());
    let expected: usize = decls.iter().map(|d| d.defined_names().len()).sum();
    assert_eq!(export.declars.len(), expected);
    // Same verdicts as the direct path: inductives fine, projection-from-Prop rejected.
    let vals: Vec<_> = export.declars.values().collect();
    for d in &vals[..6] {
        export.check_declar(d);
    }
    let bad = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export.check_declar(vals[6])));
    assert!(bad.is_err());
    assert!(NanodaKernel::default().check(&arena, &decls[..2], &decls[2..]).is_err());
}

mod robustness {
    //! Mutation tests: corrupted inputs must be rejected cleanly, never panic.
    use super::*;
    use margin_checker::{run, ProverInput};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Small deterministic PRNG (xorshift64*), so failures are reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
        let mut b = src.to_vec();
        match rng.below(4) {
            0 => {
                // flip a few bytes
                for _ in 0..1 + rng.below(4) {
                    let i = rng.below(b.len());
                    b[i] ^= 1 << rng.below(8);
                }
            }
            1 => b.truncate(rng.below(b.len())),
            2 => {
                let i = rng.below(b.len());
                b.insert(i, rng.next() as u8);
            }
            _ => {
                let i = rng.below(b.len());
                b[i] = 0xff;
            }
        }
        b
    }

    fn valid_input() -> Vec<u8> {
        let (mut arena, decls) = load("ProjFromProp");
        let env = margin_checker::Env::build(&arena, decls[..2].to_vec()).unwrap();
        let wrap = arena.cnst("Wrap", vec![]);
        let mk = arena.cnst("Wrap.mk", vec![]);
        let mk_wrap = arena.app(mk, wrap);
        let t = Decl::Theorem { name: margin_canon::Name::from_dotted("t"), lparams: vec![], ty: wrap, val: mk_wrap };
        let (env_decls, env_proof) = env.open(&arena, &[0, 1], &[]);
        ProverInput {
            env_root: env.tree.root,
            owner: [5; 32],
            self_id: [0; 32],
            module: margin_canon::Name::from_dotted("Test"),
            claimed: vec![margin_canon::Name::from_dotted("t")],
            env_decls,
            env_proof,
            deps: vec![],
            user_decls: vec![encode_decl(&arena, &t)],
        }
        .encode()
    }

    #[test]
    fn corrupted_prover_inputs_never_panic() {
        let good = valid_input();
        // Sanity: the unmodified input is accepted.
        let inp = ProverInput::decode(&good).unwrap();
        run(&inp, &mut NanodaKernel::default()).unwrap();
        let mut rng = Rng(0x5eed_1234_abcd_0001);
        let (mut decoded, mut accepted) = (0, 0);
        for _ in 0..3000 {
            let bad = mutate(&mut rng, &good);
            let r = catch_unwind(AssertUnwindSafe(|| {
                if let Ok(mut inp) = ProverInput::decode(&bad) {
                    decoded += 1;
                    if run(&inp, &mut NanodaKernel::default()).is_ok() {
                        accepted += 1;
                        // Only the owner key is free-form; any other accepted change must
                        // leave the input semantically identical (same journal).
                        let orig = ProverInput::decode(&good).unwrap();
                        let j1 = run(&orig, &mut NanodaKernel::default()).unwrap().journal().unwrap();
                        let j2 = run(&inp, &mut NanodaKernel::default()).unwrap().journal().unwrap();
                        // Free-form in claim mode without dependencies: the owner key
                        // (journal), and the image ID and module name (unused).
                        inp.owner = orig.owner;
                        inp.self_id = orig.self_id;
                        inp.module = orig.module.clone();
                        let (mut a, mut b) = (j1.clone(), j2.clone());
                        a.owner = [0; 32];
                        b.owner = [0; 32];
                        assert_eq!(a, b, "accepted mutation changed the journal beyond the owner");
                        assert!(inp == orig || j1.owner != j2.owner || inp.user_decls != orig.user_decls,
                            "accepted mutation: {}", hex::encode(&bad));
                    }
                }
            }));
            assert!(r.is_ok(), "panic on mutated input {}", hex::encode(&bad));
        }
        eprintln!("mutations: {decoded} decoded, {accepted} still accepted");
    }

    #[test]
    fn corrupted_declarations_and_journals_never_panic() {
        let (arena, decls) = load("ProjFromProp");
        let mut rng = Rng(0x0dd_ba11_cafe_0002);
        for d in &decls {
            let good = encode_decl(&arena, d);
            for _ in 0..500 {
                let bad = mutate(&mut rng, &good);
                let r = catch_unwind(AssertUnwindSafe(|| {
                    let _ = decode_decl(&bad, &mut Arena::new());
                    let _ = margin_canon::decode_decl_trusted(&bad, &mut Arena::new());
                }));
                assert!(r.is_ok(), "panic decoding {}", hex::encode(&bad));
            }
        }
        let good = margin_journal::Journal {
            version: margin_journal::VERSION,
            checker: "x".into(),
            env_root: [1; 32],
            owner: [2; 32],
            claims: vec![encode_decl(&arena, &decls[0])],
            stmt_cone: vec![],
            axioms: vec![margin_canon::Name::from_dotted("propext")],
            assumed_id: [0; 32],
        }
        .encode();
        for _ in 0..2000 {
            let bad = mutate(&mut rng, &good);
            assert!(catch_unwind(|| margin_journal::Journal::decode(&bad)).is_ok());
        }
    }
}
