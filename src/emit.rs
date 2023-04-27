// TEMP
#![allow(dead_code)]

mod regalloc;
mod stmt;

// TODO:
// 1. (optimization) constant pool compaction
// 2. register allocation
// 3. loop_header + emit_jump_loop
// 4. (optimization) basic blocks
// 5. (optimization) elide previous instruction (clobbered read)
// 6. actually write emit for all AST nodes

use beef::lean::Cow;
use indexmap::{IndexMap, IndexSet};

use self::regalloc::{RegAlloc, Register};
use crate::ctx::Context;
use crate::instruction::builder::BytecodeBuilder;
use crate::instruction::opcodes as op;
use crate::syntax::ast;
use crate::value::object;
use crate::value::object::function;
use crate::value::object::ptr::Ptr;

pub fn emit<'cx, 'src>(
  cx: &'cx Context,
  ast: &'src ast::Module<'src>,
  name: impl Into<Cow<'src, str>>,
  is_root: bool,
) -> Ptr<object::ModuleDescriptor> {
  let name = name.into();

  let mut module = State::new(cx, ast, name.clone(), is_root).emit_module();

  let name = cx.alloc(object::String::new(name.to_string().into()));
  let root = module.functions.pop().unwrap().finish(cx);
  let module_vars = module.vars;

  cx.alloc(object::ModuleDescriptor {
    name,
    root,
    module_vars,
  })
}

struct State<'cx, 'src> {
  cx: &'cx Context,
  ast: &'src ast::Module<'src>,
  module: Module<'src>,
}

impl<'cx, 'src> State<'cx, 'src> {
  fn new(
    cx: &'cx Context,
    ast: &'src ast::Module<'src>,
    name: impl Into<Cow<'src, str>>,
    is_root: bool,
  ) -> Self {
    Self {
      cx,
      ast,
      module: Module {
        is_root,
        vars: IndexSet::new(),
        functions: vec![Function::new(name, function::Params::default())],
      },
    }
  }

  fn current_function(&mut self) -> &mut Function<'src> {
    self.module.functions.last_mut().unwrap()
  }

  fn builder(&mut self) -> &mut BytecodeBuilder {
    &mut self.current_function().builder
  }

  fn emit_module(mut self) -> Module<'src> {
    for stmt in self.ast.body.iter() {
      self.emit_stmt(stmt);
    }
    self.builder().emit(op::Ret);

    self.module
  }
}

struct Module<'src> {
  is_root: bool,
  vars: IndexSet<Ptr<object::String>>,
  functions: Vec<Function<'src>>,
}

struct Function<'src> {
  name: Cow<'src, str>,
  builder: BytecodeBuilder,
  regalloc: RegAlloc,

  params: function::Params,
  locals: IndexMap<(Scope, Cow<'src, str>), Register>,
  upvalues: IndexMap<Cow<'src, str>, Upvalue>,

  is_in_opt_expr: bool,
  current_loop: Option<Loop>,
}

impl<'src> Function<'src> {
  fn new(name: impl Into<Cow<'src, str>>, params: function::Params) -> Self {
    Self {
      name: name.into(),
      builder: BytecodeBuilder::new(),
      regalloc: RegAlloc::new(),

      params,
      locals: IndexMap::new(),
      upvalues: IndexMap::new(),

      is_in_opt_expr: false,
      current_loop: None,
    }
  }

  fn finish(self, _: &Context) -> Ptr<object::FunctionDescriptor> {
    // 1. finalize regalloc
    // 2. patch instructions with register map
    // 3. allocate function descriptor

    /* cx.alloc(object::FunctionDescriptor::new(
      cx.alloc(object::String::new(self.name.to_string().into())),
      self.params,
      self.upvalues.len() as u16,
      frame_size,
      self.instructions,
      self.constants.into_iter().collect(),
    )) */
    todo!()
  }
}

enum Upvalue {
  /// Upvalue a local in the outer scope
  Parent { src: Register, dst: op::Upvalue },
  /// Upvalue an upvalue in the outer scope
  Nested { src: op::Upvalue, dst: op::Upvalue },
}

struct Loop {
  start: Label,
  end: Label,
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Label(usize);

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Scope(usize);
