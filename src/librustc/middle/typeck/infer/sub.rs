// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use core::prelude::*;

use middle::ty;
use middle::typeck::check::regionmanip::replace_bound_regions_in_fn_sig;
use middle::typeck::infer::combine::*;
use middle::typeck::infer::cres;
use middle::typeck::infer::glb::Glb;
use middle::typeck::infer::InferCtxt;
use middle::typeck::infer::lub::Lub;
use middle::typeck::infer::to_str::InferStr;
use middle::typeck::infer::unify::*;
use util::ppaux::bound_region_to_str;

use std::list::Nil;
use std::list;
use syntax::ast::{m_const, purity, ret_style};

pub fn macros() {
    // FIXME(#3114): Macro import/export.
    include!("macros.rs");
}

pub enum Sub = CombineFields;  // "subtype", "subregion" etc

pub impl Sub: Combine {
    fn infcx() -> @InferCtxt { self.infcx }
    fn tag() -> ~str { ~"sub" }
    fn a_is_expected() -> bool { self.a_is_expected }
    fn span() -> span { self.span }

    fn sub() -> Sub { Sub(*self) }
    fn lub() -> Lub { Lub(*self) }
    fn glb() -> Glb { Glb(*self) }

    fn contratys(a: ty::t, b: ty::t) -> cres<ty::t> {
        let opp = CombineFields {
            a_is_expected: !self.a_is_expected,.. *self
        };
        Sub(opp).tys(b, a)
    }

    fn contraregions(a: ty::Region, b: ty::Region) -> cres<ty::Region> {
        let opp = CombineFields {
            a_is_expected: !self.a_is_expected,.. *self
        };
        Sub(opp).regions(b, a)
    }

    fn regions(a: ty::Region, b: ty::Region) -> cres<ty::Region> {
        debug!("%s.regions(%s, %s)",
               self.tag(),
               a.inf_str(self.infcx),
               b.inf_str(self.infcx));
        do indent {
            match self.infcx.region_vars.make_subregion(self.span, a, b) {
              Ok(()) => Ok(a),
              Err(ref e) => Err((*e))
            }
        }
    }

    fn mts(a: ty::mt, b: ty::mt) -> cres<ty::mt> {
        debug!("mts(%s <: %s)", a.inf_str(self.infcx), b.inf_str(self.infcx));

        if a.mutbl != b.mutbl && b.mutbl != m_const {
            return Err(ty::terr_mutability);
        }

        match b.mutbl {
          m_mutbl => {
            // If supertype is mut, subtype must match exactly
            // (i.e., invariant if mut):
            eq_tys(&self, a.ty, b.ty).then(|| Ok(a) )
          }
          m_imm | m_const => {
            // Otherwise we can be covariant:
            self.tys(a.ty, b.ty).chain(|_t| Ok(a) )
          }
        }
    }

    fn purities(a: purity, b: purity) -> cres<purity> {
        self.lub().purities(a, b).compare(b, || {
            ty::terr_purity_mismatch(expected_found(&self, a, b))
        })
    }

    fn oncenesses(a: Onceness, b: Onceness) -> cres<Onceness> {
        self.lub().oncenesses(a, b).compare(b, || {
            ty::terr_onceness_mismatch(expected_found(&self, a, b))
        })
    }

    fn tys(a: ty::t, b: ty::t) -> cres<ty::t> {
        debug!("%s.tys(%s, %s)", self.tag(),
               a.inf_str(self.infcx), b.inf_str(self.infcx));
        if a == b { return Ok(a); }
        let _indenter = indenter();
        match (&ty::get(a).sty, &ty::get(b).sty) {
            (&ty::ty_bot, _) => {
                Ok(a)
            }

            (&ty::ty_infer(TyVar(a_id)), &ty::ty_infer(TyVar(b_id))) => {
                if_ok!(self.var_sub_var(a_id, b_id));
                Ok(a)
            }
            (&ty::ty_infer(TyVar(a_id)), _) => {
                if_ok!(self.var_sub_t(a_id, b));
                Ok(a)
            }
            (_, &ty::ty_infer(TyVar(b_id))) => {
                if_ok!(self.t_sub_var(a, b_id));
                Ok(a)
            }

            (_, &ty::ty_bot) => {
                Err(ty::terr_sorts(expected_found(&self, a, b)))
            }

            _ => {
                super_tys(&self, a, b)
            }
        }
    }

    fn fn_sigs(a: &ty::FnSig, b: &ty::FnSig) -> cres<ty::FnSig> {
        debug!("fn_sigs(a=%s, b=%s)",
               a.inf_str(self.infcx), b.inf_str(self.infcx));
        let _indenter = indenter();

        // Rather than checking the subtype relationship between `a` and `b`
        // as-is, we need to do some extra work here in order to make sure
        // that function subtyping works correctly with respect to regions
        //
        // Note: this is a subtle algorithm.  For a full explanation,
        // please see the large comment in `region_inference.rs`.

        // Take a snapshot.  We'll never roll this back, but in later
        // phases we do want to be able to examine "all bindings that
        // were created as part of this type comparison", and making a
        // snapshot is a convenient way to do that.
        let snapshot = self.infcx.region_vars.start_snapshot();

        // First, we instantiate each bound region in the subtype with a fresh
        // region variable.
        let (a_sig, _) =
            self.infcx.replace_bound_regions_with_fresh_regions(
                self.span, a);

        // Second, we instantiate each bound region in the supertype with a
        // fresh concrete region.
        let {fn_sig: b_sig, isr: skol_isr, _} = {
            do replace_bound_regions_in_fn_sig(self.infcx.tcx, @Nil,
                                              None, b) |br| {
                let skol = self.infcx.region_vars.new_skolemized(br);
                debug!("Bound region %s skolemized to %?",
                       bound_region_to_str(self.infcx.tcx, br),
                       skol);
                skol
            }
        };

        debug!("a_sig=%s", a_sig.inf_str(self.infcx));
        debug!("b_sig=%s", b_sig.inf_str(self.infcx));

        // Compare types now that bound regions have been replaced.
        let sig = if_ok!(super_fn_sigs(&self, &a_sig, &b_sig));

        // Presuming type comparison succeeds, we need to check
        // that the skolemized regions do not "leak".
        let new_vars =
            self.infcx.region_vars.vars_created_since_snapshot(snapshot);
        for list::each(skol_isr) |pair| {
            let (skol_br, skol) = *pair;
            let tainted = self.infcx.region_vars.tainted(snapshot, skol);
            for tainted.each |tainted_region| {
                // Each skolemized should only be relatable to itself
                // or new variables:
                match *tainted_region {
                    ty::re_infer(ty::ReVar(ref vid)) => {
                        if new_vars.contains(vid) { loop; }
                    }
                    _ => {
                        if *tainted_region == skol { loop; }
                    }
                };

                // A is not as polymorphic as B:
                if self.a_is_expected {
                    return Err(ty::terr_regions_insufficiently_polymorphic(
                        skol_br, *tainted_region));
                } else {
                    return Err(ty::terr_regions_overly_polymorphic(
                        skol_br, *tainted_region));
                }
            }
        }

        return Ok(sig);
    }

    // Traits please (FIXME: #2794):

    fn sigils(p1: ast::Sigil, p2: ast::Sigil) -> cres<ast::Sigil> {
        super_sigils(&self, p1, p2)
    }

    fn abis(p1: ast::Abi, p2: ast::Abi) -> cres<ast::Abi> {
        super_abis(&self, p1, p2)
    }

    fn flds(a: ty::field, b: ty::field) -> cres<ty::field> {
        super_flds(&self, a, b)
    }

    fn bare_fn_tys(a: &ty::BareFnTy,
                   b: &ty::BareFnTy) -> cres<ty::BareFnTy> {
        super_bare_fn_tys(&self, a, b)
    }

    fn closure_tys(a: &ty::ClosureTy,
                   b: &ty::ClosureTy) -> cres<ty::ClosureTy> {
        super_closure_tys(&self, a, b)
    }

    fn vstores(vk: ty::terr_vstore_kind,
               a: ty::vstore, b: ty::vstore) -> cres<ty::vstore> {
        super_vstores(&self, vk, a, b)
    }

    fn modes(a: ast::mode, b: ast::mode) -> cres<ast::mode> {
        super_modes(&self, a, b)
    }

    fn args(a: ty::arg, b: ty::arg) -> cres<ty::arg> {
        super_args(&self, a, b)
    }

    fn substs(did: ast::def_id,
              as_: &ty::substs,
              bs: &ty::substs) -> cres<ty::substs> {
        super_substs(&self, did, as_, bs)
    }

    fn tps(as_: &[ty::t], bs: &[ty::t]) -> cres<~[ty::t]> {
        super_tps(&self, as_, bs)
    }

    fn self_tys(a: Option<ty::t>, b: Option<ty::t>) -> cres<Option<ty::t>> {
        super_self_tys(&self, a, b)
    }
}

