//! Links hand-written code into plinth-rt, wraps it as a component, and runs
//! it. This proves the append linker before the codegen uses it.

use plinth_compiler::link::{self, AppCode};
use plinth_protocol::{ControlKind, prop};
use plinth_runner_wasmtime::{Limits, Runner};
use plinth_ui::tree::Tree;
use wasm_encoder::{ConstExpr, Function, Instruction as I, MemArg, ValType};

#[test]
fn linked_app_builds_a_screen() {
    let rt = link::runtime();
    let layout = link::layout(rt).expect("layout");
    let rtf = |n: &str| layout.rt(n);

    let mut app = AppCode::default();
    let thunk_ty = layout.type_count; // (i32) -> ()
    let void_ty = layout.type_count + 1; // () -> ()
    app.types.push((vec![ValType::I32], vec![]));
    app.types.push((vec![], vec![]));
    let main_idx = layout.func_count;
    let start_idx = layout.func_count + 1;
    let title = b"Hello";
    app.data.push(title.to_vec());
    let seg = layout.data_count;
    // A global that keeps the literal, so we test the global append too.
    app.globals.push((ValType::I32, ConstExpr::i32_const(0)));
    let lit_global = layout.global_count;

    // main(env): screen = node(Screen); lit = str_new(5); memory.init;
    // prop_str(screen, TITLE, lit); set_root(0, screen)
    let mut main = Function::new(vec![(1, ValType::I32)]);
    main.instruction(&I::I32Const(ControlKind::Screen as i32))
        .instruction(&I::Call(rtf("node")))
        .instruction(&I::LocalSet(1))
        .instruction(&I::I32Const(title.len() as i32))
        .instruction(&I::Call(rtf("str_new")))
        .instruction(&I::GlobalSet(lit_global))
        .instruction(&I::GlobalGet(lit_global))
        .instruction(&I::I32Const(12))
        .instruction(&I::I32Add)
        .instruction(&I::I32Const(0))
        .instruction(&I::I32Const(title.len() as i32))
        .instruction(&I::MemoryInit { mem: 0, data_index: seg })
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(prop::TITLE as i32))
        .instruction(&I::GlobalGet(lit_global))
        .instruction(&I::Call(rtf("prop_str")))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalGet(1))
        .instruction(&I::Call(rtf("set_root")))
        .instruction(&I::End);
    app.funcs.push((thunk_ty, main));
    app.table.push(main_idx);

    let mut start = Function::new(vec![]);
    if let Some(s) = layout.rt_start {
        start.instruction(&I::Call(s));
    }
    start
        .instruction(&I::I32Const(layout.table_size as i32))
        .instruction(&I::Call(rtf("set_main")))
        .instruction(&I::End);
    app.funcs.push((void_ty, start));
    app.start = Some(start_idx);
    let _ = MemArg { offset: 0, align: 2, memory_index: 0 };

    let core = link::link(rt, &layout, &app).expect("link");
    let component = link::componentize(&core).expect("componentize");

    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for c in guest.init(&[]).unwrap() {
        assert!(tree.apply(&c).unwrap().is_empty());
    }
    let root = tree.current_root().expect("a root screen");
    assert_eq!(root.kind, Some(ControlKind::Screen));
    assert_eq!(root.str_prop(prop::TITLE), Some("Hello"));
    eprintln!("runtime {} KiB, linked component {} KiB", rt.len() / 1024, component.len() / 1024);
}
