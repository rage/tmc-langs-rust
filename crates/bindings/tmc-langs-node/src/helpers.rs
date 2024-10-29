//! Contains helper functions for getting Rust types from neon arguments.

use neon::{prelude::*, result::Throw};
use serde::Serialize;
use std::error::Error;

#[macro_export]
macro_rules! lock {
    ( $cx: ident, $( $path: expr ),+ ) => {
        $(
            let path_buf: PathBuf = (&$path).into();
            let mut lock = $crate::file_util::Lock::dir(path_buf, $crate::file_util::LockOptions::Write).map_err(|e| $crate::helpers::convert_err(&mut $cx, e))?;
            let _guard = lock.lock().map_err(|e| $crate::helpers::convert_err(&mut $cx, e))?;
        )*
    };
}

#[macro_export]
macro_rules! parse_arg {
    ($cx: ident, $ty: path, $i: expr) => {{
        let arg = $cx.argument::<JsValue>($i)?;
        $crate::de::from_value::<_, $ty>(&mut $cx, arg).expect("failed to parse argument")
    }};
}

#[macro_export]
macro_rules! parse_args {
    ($cx: ident, $id0: ident : $ty0: path) => {
        let $id0: $ty0 = parse_arg!($cx, $ty0, 0);
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path) => {
        let ($id0, $id1) = (parse_arg!($cx, $ty0, 0), parse_arg!($cx, $ty1, 1));
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path) => {
        let ($id0, $id1, $id2) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path) => {
        let ($id0, $id1, $id2, $id3) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path) => {
        let ($id0, $id1, $id2, $id3, $id4) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path, $id5: ident : $ty5: path) => {
        let ($id0, $id1, $id2, $id3, $id4, $id5) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
            parse_arg!($cx, $ty5, 5),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path, $id5: ident : $ty5: path, $id6: ident : $ty6: path) => {
        let ($id0, $id1, $id2, $id3, $id4, $id5, $id6) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
            parse_arg!($cx, $ty5, 5),
            parse_arg!($cx, $ty6, 6),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path, $id5: ident : $ty5: path, $id6: ident : $ty6: path, $id7: ident : $ty7: path) => {
        let ($id0, $id1, $id2, $id3, $id4, $id5, $id6, $id7) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
            parse_arg!($cx, $ty5, 5),
            parse_arg!($cx, $ty6, 6),
            parse_arg!($cx, $ty7, 7),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path, $id5: ident : $ty5: path, $id6: ident : $ty6: path, $id7: ident : $ty7: path, $id8: ident : $ty8: path) => {
        let ($id0, $id1, $id2, $id3, $id4, $id5, $id6, $id7, $id8) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
            parse_arg!($cx, $ty5, 5),
            parse_arg!($cx, $ty6, 6),
            parse_arg!($cx, $ty7, 7),
            parse_arg!($cx, $ty8, 8),
        );
    };
    ($cx: ident, $id0: ident : $ty0: path, $id1: ident : $ty1: path, $id2: ident : $ty2: path, $id3: ident : $ty3: path, $id4: ident : $ty4: path, $id5: ident : $ty5: path, $id6: ident : $ty6: path, $id7: ident : $ty7: path, $id8: ident : $ty8: path, $id9: ident : $ty9: path) => {
        let ($id0, $id1, $id2, $id3, $id4, $id5, $id6, $id7, $id8, $id9) = (
            parse_arg!($cx, $ty0, 0),
            parse_arg!($cx, $ty1, 1),
            parse_arg!($cx, $ty2, 2),
            parse_arg!($cx, $ty3, 3),
            parse_arg!($cx, $ty4, 4),
            parse_arg!($cx, $ty5, 5),
            parse_arg!($cx, $ty6, 6),
            parse_arg!($cx, $ty7, 7),
            parse_arg!($cx, $ty8, 8),
            parse_arg!($cx, $ty9, 9),
        );
    };
}

pub fn convert_err<E: Error>(cx: &mut FunctionContext, e: E) -> Throw {
    let mut trace = vec![e.to_string()];
    let mut source = e.source();
    while let Some(s) = source {
        trace.push(s.to_string());
        source = s.source();
    }
    let err = crate::ser::to_value(cx, &trace).expect("failed to convert error");
    cx.throw::<_, ()>(err).expect_err("should never happen")
}

pub fn convert_res<'a, T: Serialize, E: Error>(
    cx: &mut FunctionContext<'a>,
    res: Result<T, E>,
) -> Result<Handle<'a, JsValue>, Throw> {
    res.map_err(|e| convert_err(cx, e))
        .and_then(|t| convert(cx, &t))
}

pub fn convert<'a, T: Serialize>(
    cx: &mut FunctionContext<'a>,
    t: &T,
) -> Result<Handle<'a, JsValue>, Throw> {
    crate::ser::to_value(cx, &t).map_err(|e| convert_err(cx, e))
}
