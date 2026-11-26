//! [`Kernel`] backed by nanoda_lib (vendored in `third_party/nanoda_lib`).
//!
//! The adapter emits the declarations as lean4export records and feeds them, already
//! typed, to nanoda's own record processor (`Parser::add_record` / `Parser::finish`).
//! nanoda's validation (back-references, inductive blocks, recursor ownership) runs
//! exactly as for a file; only the JSON text step is skipped.
//! Only the user declarations are type-checked; the environment is trusted.

use margin_canon::lean4export::{export_records, ExportError, ExprRec, Record};
use margin_canon::{Arena, BinderInfo, Decl, Hints, QuotKind, Safety};
use nanoda_lib::env::ReducibilityHint;
use nanoda_lib::expr::BinderStyle;
use nanoda_lib::parser::{
    BackRef, Constructor, DefinitionSafety, ExportJsonObject, ExportJsonVal, ExporterMeta, FileMeta, FormatMeta,
    IndInfo, LeanMeta, Parser, QuotKind as NQuotKind, Recursor, RecursorRule,
};
use std::borrow::Cow;
use margin_checker::{Kernel, KernelError};
use nanoda_lib::util::Config;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Name written to the journal.
pub const NAME: &str = "nanoda_lib 0.4.32 @ 0da9f98";

/// Axioms are not filtered here: `margin-checker` enforces the axiom policy on claims.
/// Nat and String literal extensions match the official Lean kernel.
const CONFIG: &str = r#"{
    "use_stdin": true,
    "unsafe_permit_all_axioms": true,
    "unpermitted_axiom_hard_error": false,
    "nat_extension": true,
    "string_extension": true,
    "num_threads": 1,
    "print_success_message": false,
    "print_axioms": false
}"#;

#[derive(Default)]
pub struct NanodaKernel {
    /// Optional profiling hook, called with a phase name at each phase boundary.
    pub phase_hook: Option<fn(&str)>,
    /// Record which environment declarations the kernel consults for their value or
    /// reducibility hint (needs the `record` feature; native only). The result, as
    /// indices into the `env` slice of the last `check`, goes to `value_uses`.
    pub record_value_uses: bool,
    pub value_uses: Vec<usize>,
}

impl NanodaKernel {
    fn mark(&self, phase: &str) {
        if let Some(h) = self.phase_hook {
            h(phase);
        }
    }
}

impl Kernel for NanodaKernel {
    fn name(&self) -> &str {
        NAME
    }

    fn check(&mut self, arena: &Arena, env: &[Decl], user: &[Decl]) -> Result<(), KernelError> {
        self.mark("kernel: start");
        let all: Vec<&Decl> = env.iter().chain(user).collect();
        // nanoda flattens an inductive block into types, constructors and recursors,
        // in the same order as `Decl::defined_names`.
        let env_len: usize = env.iter().map(|d| d.defined_names().len()).sum();
        let user_len: usize = user.iter().map(|d| d.defined_names().len()).sum();

        // nanoda reports type errors by panicking.
        let res = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
            let config: Config = serde_json::from_str(CONFIG).map_err(|e| e.to_string())?;
            let mut parser = Parser::new(std::io::empty(), config);
            let meta = FileMeta {
                lean: LeanMeta { version: Cow::Borrowed(""), githash: Cow::Borrowed("") },
                exporter: ExporterMeta { name: Cow::Borrowed("ce"), version: Cow::Borrowed("1") },
                format: FormatMeta { version: Cow::Borrowed(margin_canon::lean4export::FORMAT_VERSION) },
            };
            parser
                .add_record(ExportJsonObject { val: ExportJsonVal::Metadata(meta), i: None }, "meta")
                .map_err(|e| e.to_string())?;
            let mut feed = |r: Record| -> Result<(), ExportError> {
                let obj = to_nanoda(r)?;
                parser.add_record(obj, "").map_err(|e| ExportError(e.to_string()))
            };
            export_records(arena, &all, &mut feed).map_err(|e| e.to_string())?;
            self.mark("kernel: records fed to nanoda");
            let (export, skipped) = parser.finish().map_err(|e| e.to_string())?;
            self.mark("kernel: nanoda parsed");
            if !skipped.is_empty() {
                return Err(format!("nanoda skipped declarations: {skipped:?}"));
            }
            if export.declars.len() != env_len + user_len {
                return Err(format!(
                    "declaration count mismatch: nanoda has {}, expected {}",
                    export.declars.len(),
                    env_len + user_len
                ));
            }
            #[cfg(feature = "record")]
            if self.record_value_uses {
                nanoda_lib::env::value_uses::start();
            }
            #[cfg(not(feature = "record"))]
            if self.record_value_uses {
                return Err("value-use recording needs the `record` feature".into());
            }
            for (_, d) in export.declars.iter().skip(env_len) {
                export.check_declar(d);
            }
            #[cfg(feature = "record")]
            if self.record_value_uses {
                // nanoda's declaration index → index of the env declaration that defines it.
                let owner: Vec<usize> =
                    env.iter().enumerate().flat_map(|(i, d)| std::iter::repeat_n(i, d.defined_names().len())).collect();
                let mut uses: Vec<usize> = nanoda_lib::env::value_uses::take()
                    .into_iter()
                    .filter_map(|idx| owner.get(idx).copied())
                    .collect();
                uses.dedup();
                self.value_uses = uses;
            }
            self.mark("kernel: nanoda checked");
            Ok(())
        }));
        match res {
            Ok(r) => r.map_err(KernelError),
            Err(p) => Err(KernelError(panic_message(&p))),
        }
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        format!("type error: {s}")
    } else if let Some(s) = p.downcast_ref::<String>() {
        format!("type error: {s}")
    } else {
        "type error (no message)".into()
    }
}

fn u16_of(v: u64, what: &str) -> Result<u16, ExportError> {
    u16::try_from(v).map_err(|_| ExportError(format!("{what} exceeds u16")))
}

fn style(bi: BinderInfo) -> BinderStyle {
    match bi {
        BinderInfo::Default => BinderStyle::Default,
        BinderInfo::Implicit => BinderStyle::Implicit,
        BinderInfo::StrictImplicit => BinderStyle::StrictImplicit,
        BinderInfo::InstImplicit => BinderStyle::InstanceImplicit,
    }
}

/// One record → the object nanoda's parser would have deserialized from the same line.
fn to_nanoda<'a>(r: Record<'a>) -> Result<ExportJsonObject<'a>, ExportError> {
    use ExportJsonVal as V;
    let (val, i) = match r {
        Record::NameStr { idx, pre, s } => (V::NameStr { pre, str: Cow::Borrowed(s) }, Some(BackRef::In(idx))),
        Record::NameNum { idx, pre, i } => (V::NameNum { pre, i }, Some(BackRef::In(idx))),
        Record::Succ { idx, l } => (V::LevelSucc(l), Some(BackRef::Il(idx))),
        Record::Max { idx, a, b } => (V::LevelMax([a, b]), Some(BackRef::Il(idx))),
        Record::IMax { idx, a, b } => (V::LevelIMax([a, b]), Some(BackRef::Il(idx))),
        Record::Param { idx, name } => (V::LevelParam(name), Some(BackRef::Il(idx))),
        Record::Expr { idx, e } => {
            let v = match e {
                ExprRec::BVar(i) => V::ExprBVar(i),
                ExprRec::Sort(l) => V::ExprSort(l),
                ExprRec::Const { name, us } => V::ExprConst { name, levels: us },
                ExprRec::App { f, a } => V::ExprApp { fun: f, arg: a },
                ExprRec::Lam { name, ty, body, bi } => {
                    V::ExprLambda { binder_name: name, binder_type: ty, body, binder_info: style(bi) }
                }
                ExprRec::Pi { name, ty, body, bi } => {
                    V::ExprPi { binder_name: name, binder_type: ty, body, binder_info: style(bi) }
                }
                ExprRec::Let { name, ty, val, body, nondep } => V::ExprLet { name, ty, value: val, body, nondep },
                ExprRec::NatLit(b) => V::NatLit(num_bigint::BigUint::from_bytes_be(b)),
                ExprRec::StrLit(s) => V::StrLit(Cow::Borrowed(s)),
                ExprRec::Proj { type_name, idx, structure } => V::ExprProj {
                    type_name,
                    idx: usize::try_from(idx).map_err(|_| ExportError("projection index too large".into()))?,
                    structure,
                },
            };
            (v, Some(BackRef::Ie(idx)))
        }
        Record::Axiom { name, lparams, ty, is_unsafe } => (V::Axiom { name, uparams: lparams, ty, is_unsafe }, None),
        Record::Thm { name, lparams, ty, val } => (V::Thm { name, uparams: lparams, ty, value: val }, None),
        Record::Def { name, lparams, ty, val, hints, safety } => (
            V::Defn {
                name,
                uparams: lparams,
                ty,
                value: val,
                hint: match hints {
                    Hints::Opaque => ReducibilityHint::Opaque,
                    Hints::Abbrev => ReducibilityHint::Abbrev,
                    Hints::Regular(h) => ReducibilityHint::Regular(u16_of(h as u64, "reducibility height")?),
                },
                safety: match safety {
                    Safety::Safe => DefinitionSafety::Safe,
                    Safety::Unsafe => DefinitionSafety::Unsafe,
                    Safety::Partial => DefinitionSafety::Partial,
                },
            },
            None,
        ),
        Record::Opaque { name, lparams, ty, val, is_unsafe } => {
            (V::Opaque { name, uparams: lparams, ty, value: val, is_unsafe }, None)
        }
        Record::Quot { name, lparams, ty, kind } => (
            V::Quot {
                name,
                uparams: lparams,
                ty,
                kind: match kind {
                    QuotKind::Type => NQuotKind::Ty,
                    QuotKind::Ctor => NQuotKind::Ctor,
                    QuotKind::Lift => NQuotKind::Lift,
                    QuotKind::Ind => NQuotKind::Ind,
                },
            },
            None,
        ),
        Record::Inductive { types, ctors, recs } => {
            let ind_vals = types
                .into_iter()
                .map(|t| {
                    Ok(IndInfo {
                        name: t.name,
                        uparams: t.lparams,
                        ty: t.ty,
                        all: t.all,
                        ctors: t.ctors,
                        is_rec: t.is_rec,
                        is_reflexive: t.is_reflexive,
                        num_indices: u16_of(t.num_indices, "numIndices")?,
                        num_nested: u16_of(t.num_nested, "numNested")?,
                        num_params: u16_of(t.num_params, "numParams")?,
                        is_unsafe: t.is_unsafe,
                    })
                })
                .collect::<Result<_, ExportError>>()?;
            let ctor_vals = ctors
                .into_iter()
                .map(|c| {
                    Ok(Constructor {
                        name: c.name,
                        uparams: c.lparams,
                        ty: c.ty,
                        is_unsafe: c.is_unsafe,
                        cidx: u16_of(c.cidx, "cidx")?,
                        num_params: u16_of(c.num_params, "numParams")?,
                        num_fields: u16_of(c.num_fields, "numFields")?,
                        induct: c.induct,
                    })
                })
                .collect::<Result<_, ExportError>>()?;
            let rec_vals = recs
                .into_iter()
                .map(|r| {
                    Ok(Recursor {
                        name: r.name,
                        uparams: r.lparams,
                        ty: r.ty,
                        is_unsafe: r.is_unsafe,
                        num_params: u16_of(r.num_params, "numParams")?,
                        num_indices: u16_of(r.num_indices, "numIndices")?,
                        num_motives: u16_of(r.num_motives, "numMotives")?,
                        num_minors: u16_of(r.num_minors, "numMinors")?,
                        rules: r
                            .rules
                            .into_iter()
                            .map(|x| {
                                Ok(RecursorRule { ctor: x.ctor, nfields: u16_of(x.nfields, "nfields")?, rhs: x.rhs })
                            })
                            .collect::<Result<_, ExportError>>()?,
                        all: r.all,
                        k: r.k,
                    })
                })
                .collect::<Result<_, ExportError>>()?;
            (V::Inductive { ind_vals, ctor_vals, rec_vals }, None)
        }
    };
    Ok(ExportJsonObject { val, i })
}
