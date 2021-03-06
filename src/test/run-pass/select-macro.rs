// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// xfail-test - this isn't really a test.

 {

// select!
macro_rules! select_if (

    {
        $index:expr,
        $count:expr
    } => {
        die!()
    };

    {
        $index:expr,
        $count:expr,
        $port:path => [
            $(type_this $message:path$(($(x $x: ident),+))dont_type_this*
              -> $next:ident => { move $e:expr }),+
        ]
        $(, $ports:path => [
            $(type_this $messages:path$(($(x $xs: ident),+))dont_type_this*
              -> $nexts:ident => { move $es:expr }),+
        ] )*
    } => {
        if $index == $count {
            match move pipes::try_recv($port) {
              $(Some($message($($(move $x,)+)* move next)) => {
                let $next = move next;
                move $e
              })+
              _ => die!()
            }
        } else {
            select_if!(
                $index,
                $count + 1
                $(, $ports => [
                    $(type_this $messages$(($(x $xs),+))dont_type_this*
                      -> $nexts => { move $es }),+
                ])*
            )
        }
    };
)

macro_rules! select (
    {
        $( $port:path => {
            $($message:path$(($($x: ident),+))dont_type_this*
              -> $next:ident $e:expr),+
        } )+
    } => {
        let index = pipes::selecti([$(($port).header()),+]);
        select_if!(index, 0 $(, $port => [
            $(type_this $message$(($(x $x),+))dont_type_this* -> $next => { move $e }),+
        ])+)
    }
)

}
