#[macro_use]
mod macros;

mod util;

use std::cell::RefCell;
use std::fmt::{Debug, Display};
use std::mem::take;
use std::ops::Deref;
use std::ptr::NonNull;
use std::rc::Rc;

use indexmap::IndexMap;

use self::util::*;
use super::dispatch;
use super::dispatch::{dispatch, ControlFlow, Handler};
use super::global::Global;
use crate as hebi;
use crate::bytecode::opcode as op;
use crate::ctx::Context;
use crate::object::class::{ClassInstance, ClassMethod, ClassProxy};
use crate::object::function::Params;
use crate::object::module::ModuleId;
use crate::object::{
  ClassDescriptor, ClassType, Function, FunctionDescriptor, List, Module, Object, Ptr, String,
  Table,
};
use crate::value::constant::Constant;
use crate::value::Value;
use crate::{emit, object, syntax, Error};

pub struct Thread {
  cx: Context,
  global: Global,

  // TODO: `Stack` behind Rc+RefCell
  // - stored call_frames, stack, stack_base
  // eventually should be a flat buffer
  call_frames: Rc<RefCell<Vec<Frame>>>,
  stack: Ptr<List>,
  stack_base: usize,
  acc: Value,
  pc: usize,
}

impl Thread {
  pub fn new(cx: Context, global: Global) -> Self {
    Thread {
      cx: cx.clone(),
      global,

      call_frames: Rc::new(RefCell::new(Vec::new())),
      stack: cx.alloc(List::with_capacity(128)),
      stack_base: 0,
      acc: Value::none(),
      pc: 0,
    }
  }

  pub fn call(&mut self, f: Value, args: &[Value]) -> hebi::Result<Value> {
    let (stack_base, num_args) = push_args!(self, args);
    self.do_call(f, stack_base, num_args, None)?;
    self.run()?;
    Ok(take(&mut self.acc))
  }

  fn run(&mut self) -> hebi::Result<()> {
    let instructions = current_call_frame_mut!(self).instructions;
    let pc = self.pc;

    match dispatch(self, instructions, pc)? {
      ControlFlow::Yield(pc) => {
        self.pc = pc;
        Ok(())
      }
      ControlFlow::Return => {
        self.pc = 0;
        Ok(())
      }
    }
  }

  fn do_call(
    &mut self,
    value: Value,
    stack_base: usize,
    num_args: usize,
    return_addr: Option<usize>,
  ) -> hebi::Result<dispatch::Call> {
    let object = match value.try_to_any() {
      Ok(f) => f,
      Err(f) => hebi::fail!("cannot call value `{f}`"),
    };

    if object.is::<Function>() {
      let function = unsafe { object.cast_unchecked::<Function>() };
      self.call_function(function, stack_base, num_args, return_addr)
    } else if object.is::<ClassMethod>() {
      let method = unsafe { object.cast_unchecked::<ClassMethod>() };
      self.call_method(method, stack_base, num_args, return_addr)
    } else if object.is::<ClassType>() {
      let class = unsafe { object.cast_unchecked::<ClassType>() };
      self.init_class(class, stack_base, num_args)
    } else {
      hebi::fail!("cannot call object `{object}`")
    }
  }

  fn call_function(
    &mut self,
    function: Ptr<Function>,
    stack_base: usize,
    num_args: usize,
    return_addr: Option<usize>,
  ) -> hebi::Result<dispatch::Call> {
    check_args(&function.descriptor.params, false, num_args)?;

    self.pc = 0;
    self.stack_base = stack_base;
    self
      .stack
      .extend(stack_base + function.descriptor.frame_size);

    self.call_frames.borrow_mut().push(Frame {
      instructions: function.descriptor.instructions,
      constants: function.descriptor.constants,
      upvalues: function.upvalues.clone(),
      frame_size: function.descriptor.frame_size,
      return_addr,
      module_id: function.module_id,
    });

    Ok(
      dispatch::LoadFrame {
        bytecode: function.descriptor.instructions,
        pc: 0,
      }
      .into(),
    )
  }

  fn call_method(
    &mut self,
    method: Ptr<ClassMethod>,
    stack_base: usize,
    num_args: usize,
    return_addr: Option<usize>,
  ) -> hebi::Result<dispatch::Call> {
    let function = unsafe { method.function().cast_unchecked::<Function>() };
    check_args(&function.descriptor.params, true, num_args)?;

    self.pc = 0;
    self.stack_base = stack_base;
    self
      .stack
      .extend(stack_base + function.descriptor.frame_size);
    self.set_register(op::Register(0), Value::object(method.this()));

    self.call_frames.borrow_mut().push(Frame {
      instructions: function.descriptor.instructions,
      constants: function.descriptor.constants,
      upvalues: function.upvalues.clone(),
      frame_size: function.descriptor.frame_size,
      return_addr,
      module_id: function.module_id,
    });

    Ok(
      dispatch::LoadFrame {
        bytecode: function.descriptor.instructions,
        pc: 0,
      }
      .into(),
    )
  }

  fn init_class(
    &mut self,
    class: Ptr<ClassType>,
    stack_base: usize,
    num_args: usize,
  ) -> hebi::Result<dispatch::Call> {
    let instance = self.cx.alloc(ClassInstance::new(&self.cx, &class));

    if let Some(init) = class.init.as_ref() {
      check_args(&init.descriptor.params, true, num_args)?;

      // args are already on the stack (pushed by `op_call`)
      let method = self.cx.alloc(ClassMethod::new(
        instance.clone().into_any(),
        init.clone().into_any(),
      ));
      self.call_method(method, stack_base, num_args, None)?;
      self.run()?;
    } else if num_args > 0 {
      hebi::fail!("expected at most 0 args");
    }

    instance.is_frozen.set(true);

    self.acc = Value::object(instance);

    Ok(dispatch::Call::Continue)
  }

  fn make_fn(&mut self, desc: Ptr<FunctionDescriptor>) -> Ptr<Function> {
    let num_upvalues = desc.upvalues.borrow().len();
    let mut upvalues = Vec::with_capacity(num_upvalues);
    upvalues.resize_with(num_upvalues, Value::none);
    for (i, upvalue) in desc.upvalues.borrow().iter().enumerate() {
      let value = match upvalue {
        crate::object::function::Upvalue::Register(register) => self.get_register(*register),
        crate::object::function::Upvalue::Upvalue(upvalue) => {
          let parent_upvalues = &current_call_frame!(self).upvalues;
          debug_assert!(upvalue.index() < parent_upvalues.len());
          unsafe { parent_upvalues.get_unchecked(upvalue.index()) }
        }
      };
      let slot = unsafe { upvalues.get_unchecked_mut(i) };
      *slot = value;
    }
    let upvalues = self.cx.alloc(List::from(upvalues));

    self.cx.alloc(Function::new(
      desc,
      upvalues,
      current_call_frame!(self).module_id,
    ))
  }

  fn make_class(
    &mut self,
    desc: Ptr<ClassDescriptor>,
    fields: Option<Ptr<Table>>,
    parent: Option<Ptr<ClassType>>,
  ) -> Ptr<ClassType> {
    let mut init = None;
    let fields = fields.unwrap_or_else(|| self.cx.alloc(Table::new()));
    let mut methods = IndexMap::with_capacity(desc.methods.len());
    for (key, desc) in desc.methods.iter() {
      let method = self.make_fn(desc.clone());
      if key == &"init" {
        init = Some(method.clone());
      }
      methods.insert(key.clone(), method);
    }
    self.cx.alloc(ClassType::new(
      desc.name.clone(),
      init,
      fields,
      methods,
      parent,
    ))
  }

  fn load_module(&mut self, path: Ptr<String>) -> hebi::Result<Ptr<Module>> {
    if let Some((module_id, module)) = self.global.module_registry().get_by_name(path.as_str()) {
      // module is in cache
      if self.global.module_visited_set().contains(&module_id) {
        hebi::fail!("attempted to import partially initialized module {path}");
      }
      return Ok(module);
    }

    // module is not in cache, actually load it
    let module_id = self.global.module_registry_mut().next_module_id();
    // TODO: native modules
    let module = self.global.module_loader().load(path.as_str())?.to_string();
    let module = syntax::parse(&self.cx, &module).map_err(Error::Syntax)?;
    let module = emit::emit(&self.cx, &module, path.as_str(), false);
    println!("{}", module.root.disassemble());
    let main = self.cx.alloc(Function::new(
      module.root.clone(),
      self.cx.alloc(List::new()),
      module_id,
    ));
    let module = self.cx.alloc(Module::new(
      &self.cx,
      path.clone(),
      main,
      &module.module_vars,
      module_id,
    ));
    self
      .global
      .module_registry_mut()
      .insert(module_id, path, module.clone());
    self.global.module_visited_set_mut().insert(module_id);

    let result = match self.call(Value::object(module.root.clone()), &[]) {
      Ok(_) => Ok(module),
      Err(e) => {
        self.global.module_registry_mut().remove(module_id);
        Err(e)
      }
    };
    self.global.module_visited_set_mut().remove(&module_id);
    result
  }
}

impl Display for Thread {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "<thread>")
  }
}

impl Debug for Thread {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Thread")
      .field("stack", &self.stack)
      .field("acc", &self.acc)
      .field("pc", &self.pc)
      .finish()
  }
}

impl Object for Thread {
  fn type_name(&self) -> &'static str {
    "Thread"
  }
}

struct Frame {
  instructions: NonNull<[u8]>,
  constants: NonNull<[Constant]>,
  upvalues: Ptr<List>,
  frame_size: usize,
  return_addr: Option<usize>,
  module_id: ModuleId,
}

impl Thread {
  fn get_constant(&self, idx: op::Constant) -> Constant {
    clone_from_raw_slice(current_call_frame!(self).constants.as_ptr(), idx.index())
  }

  fn get_constant_object<T: Object>(&self, idx: op::Constant) -> Ptr<T> {
    let object = self.get_constant(idx).into_value();
    unsafe { object.to_any_unchecked().cast_unchecked::<T>() }
  }

  // TODO: get_register_as
  fn get_register(&self, reg: op::Register) -> Value {
    debug_assert!(
      self.stack_base + reg.index() < self.stack.len(),
      "register out of bounds {reg:?}"
    );
    unsafe { self.stack.get_unchecked(self.stack_base + reg.index()) }
  }

  fn set_register(&mut self, reg: op::Register, value: Value) {
    debug_assert!(
      self.stack_base + reg.index() < self.stack.len(),
      "register out of bounds {reg:?}"
    );
    unsafe {
      self
        .stack
        .set_unchecked(self.stack_base + reg.index(), value)
    };
  }
}

impl Handler for Thread {
  type Error = crate::vm::Error;

  fn op_load(&mut self, reg: op::Register) -> hebi::Result<()> {
    self.acc = self.get_register(reg);
    println!("load {reg} {}", self.acc);

    Ok(())
  }

  fn op_store(&mut self, reg: op::Register) -> hebi::Result<()> {
    let value = take(&mut self.acc);
    self.set_register(reg, value);

    Ok(())
  }

  fn op_load_const(&mut self, idx: op::Constant) -> hebi::Result<()> {
    self.acc = self.get_constant(idx).into_value();

    Ok(())
  }

  fn op_load_upvalue(&mut self, idx: op::Upvalue) -> hebi::Result<()> {
    let call_frame = current_call_frame!(self);
    let upvalues = &call_frame.upvalues;
    debug_assert!(
      idx.index() < upvalues.len(),
      "upvalue index is out of bounds {idx:?}"
    );
    self.acc = unsafe { call_frame.upvalues.get_unchecked(idx.index()) };

    Ok(())
  }

  fn op_store_upvalue(&mut self, idx: op::Upvalue) -> hebi::Result<()> {
    let call_frame = current_call_frame!(self);
    let upvalues = &call_frame.upvalues;
    debug_assert!(
      idx.index() < upvalues.len(),
      "upvalue index is out of bounds {idx:?}"
    );
    let value = take(&mut self.acc);
    unsafe { call_frame.upvalues.set_unchecked(idx.index(), value) };

    Ok(())
  }

  fn op_load_module_var(&mut self, idx: op::ModuleVar) -> hebi::Result<()> {
    let module_id = current_call_frame!(self).module_id;
    let module = match self.global.module_registry().get_by_id(module_id) {
      Some(module) => module,
      None => {
        hebi::fail!("failed to get module {module_id}");
      }
    };

    let value = match module.module_vars.get_index(idx.index()) {
      Some(value) => value,
      None => {
        hebi::fail!("failed to get module variable {idx}");
      }
    };

    self.acc = value;

    Ok(())
  }

  fn op_store_module_var(&mut self, idx: op::ModuleVar) -> hebi::Result<()> {
    let module_id = current_call_frame!(self).module_id;
    let module = match self.global.module_registry().get_by_id(module_id) {
      Some(module) => module,
      None => {
        hebi::fail!("failed to get module {module_id}");
      }
    };

    let value = take(&mut self.acc);

    let success = module.module_vars.set_index(idx.index(), value.clone());
    if !success {
      hebi::fail!("failed to set module variable {idx} (value={value})");
    };

    Ok(())
  }

  fn op_load_global(&mut self, name: op::Constant) -> hebi::Result<()> {
    let name = self.get_constant_object::<String>(name);
    let value = match self.global.globals().get(&name) {
      Some(value) => value,
      None => hebi::fail!("undefined global {name}"),
    };
    self.acc = value;

    Ok(())
  }

  fn op_store_global(&mut self, name: op::Constant) -> hebi::Result<()> {
    let name = self.get_constant_object::<String>(name);
    let value = take(&mut self.acc);
    self.global.globals().insert(name, value);

    Ok(())
  }

  fn op_load_field(&mut self, name: op::Constant) -> hebi::Result<()> {
    let name = self.get_constant_object::<String>(name);
    let receiver = take(&mut self.acc);
    println!("{receiver:?}");

    let value = if let Some(object) = receiver.clone().to_any() {
      match object.named_field(&self.cx, name.clone())? {
        Some(value) => value,
        None => hebi::fail!("failed to get field `{name}` on value `{object}`"),
      }
    } else {
      // TODO: fields on primitives
      todo!()
    };

    if let (Some(object), Some(value)) = (receiver.to_any(), value.clone().to_any()) {
      if object::is_class(&object) && object::is_callable(&value) {
        self.acc = Value::object(self.cx.alloc(ClassMethod::new(object, value)));
        return Ok(());
      }
    }

    self.acc = value;

    Ok(())
  }

  fn op_load_field_opt(&mut self, name: op::Constant) -> hebi::Result<()> {
    let name = self.get_constant_object::<String>(name);
    let receiver = take(&mut self.acc);

    if receiver.is_none() {
      self.acc = Value::none();
      return Ok(());
    }

    let value = if let Some(object) = receiver.clone().to_any() {
      match object.named_field(&self.cx, name)? {
        Some(value) => value,
        None => Value::none(),
      }
    } else {
      // TODO: fields on primitives
      todo!()
    };

    if let (Some(object), Some(value)) = (receiver.to_any(), value.clone().to_any()) {
      if object::is_class(&object) && object::is_callable(&value) {
        self.acc = Value::object(self.cx.alloc(ClassMethod::new(object, value)));
        return Ok(());
      }
    }

    self.acc = value;

    Ok(())
  }

  fn op_store_field(&mut self, obj: op::Register, name: op::Constant) -> hebi::Result<()> {
    let name = self.get_constant_object::<String>(name);
    let object = self.get_register(obj);
    let value = take(&mut self.acc);

    if let Some(object) = object.to_any() {
      object.set_named_field(&self.cx, name, value)?;
    } else {
      // TODO: fields on primitives
      todo!()
    }

    Ok(())
  }

  fn op_load_index(&mut self, obj: op::Register) -> hebi::Result<()> {
    let object = self.get_register(obj);
    let key = take(&mut self.acc);

    let value = if let Some(object) = object.to_any() {
      match object.keyed_field(&self.cx, key.clone())? {
        Some(value) => value,
        None => hebi::fail!("failed to get field `{key}` on value `{object}`"),
      }
    } else {
      // TODO: fields on primitives
      todo!()
    };

    self.acc = value;

    Ok(())
  }

  fn op_load_index_opt(&mut self, obj: op::Register) -> hebi::Result<()> {
    let object = self.get_register(obj);
    let key = take(&mut self.acc);

    if object.is_none() {
      self.acc = Value::none();
      return Ok(());
    }

    let value = if let Some(object) = object.to_any() {
      match object.keyed_field(&self.cx, key)? {
        Some(value) => value,
        None => Value::none(),
      }
    } else {
      // TODO: fields on primitives
      todo!()
    };

    self.acc = value;

    Ok(())
  }

  fn op_store_index(&mut self, obj: op::Register, key: op::Register) -> hebi::Result<()> {
    let object = self.get_register(obj);
    let key = self.get_register(key);
    let value = take(&mut self.acc);

    if let Some(object) = object.to_any() {
      object.set_keyed_field(&self.cx, key, value)?;
    } else {
      // TODO: fields on primitives
      todo!()
    }

    Ok(())
  }

  fn op_load_self(&mut self) -> hebi::Result<()> {
    let this = self.get_register(op::Register(0));

    let this = match this.try_to_object::<ClassProxy>() {
      Ok(proxy) => Value::object(proxy.this.clone()),
      Err(value) => value,
    };

    self.acc = this;
    Ok(())
  }

  fn op_load_super(&mut self) -> hebi::Result<()> {
    let this = self.get_register(op::Register(0));

    let Some(this) = this.to_any() else {
      hebi::fail!("`self` is not a class instance");
    };

    let proxy = if let Some(proxy) = this.clone_cast::<ClassProxy>() {
      ClassProxy {
        this: proxy.this.clone(),
        class: proxy.class.parent.clone().unwrap(),
      }
    } else if let Some(this) = this.clone_cast::<ClassInstance>() {
      ClassProxy {
        this: this.clone(),
        class: this.parent.clone().unwrap(),
      }
    } else {
      hebi::fail!("{this} is not a class");
    };

    self.acc = Value::object(self.cx.alloc(proxy));

    Ok(())
  }

  fn op_load_none(&mut self) -> hebi::Result<()> {
    self.acc = Value::none();

    Ok(())
  }

  fn op_load_true(&mut self) -> hebi::Result<()> {
    self.acc = Value::bool(true);

    Ok(())
  }

  fn op_load_false(&mut self) -> hebi::Result<()> {
    self.acc = Value::bool(false);

    Ok(())
  }

  fn op_load_smi(&mut self, smi: op::Smi) -> hebi::Result<()> {
    self.acc = Value::int(smi.value());

    Ok(())
  }

  fn op_make_fn(&mut self, desc: op::Constant) -> hebi::Result<()> {
    let desc = self.get_constant_object::<FunctionDescriptor>(desc);

    // fetch upvalues
    let f = self.make_fn(desc);

    self.acc = Value::object(f);

    Ok(())
  }

  fn op_make_class(&mut self, desc: op::Constant) -> hebi::Result<()> {
    let desc = self.get_constant_object::<ClassDescriptor>(desc);

    let class = self.make_class(desc, None, None);

    self.acc = Value::object(class);

    Ok(())
  }

  fn op_make_class_derived(&mut self, desc: op::Constant) -> hebi::Result<()> {
    let desc = self.get_constant_object::<ClassDescriptor>(desc);
    let parent = take(&mut self.acc);

    let Some(parent) = parent.clone().to_object::<ClassType>() else {
      hebi::fail!("{parent} is not a class");
    };
    let fields = self.cx.alloc(parent.fields.deref().clone());
    let class = self.make_class(desc, Some(fields), Some(parent));

    self.acc = Value::object(class);

    Ok(())
  }

  fn op_make_data_class(&mut self, desc: op::Constant, parts: op::Register) -> hebi::Result<()> {
    let desc = self.get_constant_object::<ClassDescriptor>(desc);

    let fields = self.cx.alloc(Table::with_capacity(desc.fields.len()));
    for (offset, key) in desc.fields.keys().enumerate() {
      let value = self.get_register(parts.offset(offset));
      fields.insert(key, value);
    }
    let class = self.make_class(desc, Some(fields), None);

    self.acc = Value::object(class);

    Ok(())
  }

  fn op_make_data_class_derived(
    &mut self,
    desc: op::Constant,
    parts: op::Register,
  ) -> hebi::Result<()> {
    let desc = self.get_constant_object::<ClassDescriptor>(desc);
    let parent = self.get_register(parts);

    let Some(parent) = parent.clone().to_object::<ClassType>() else {
      hebi::fail!("{parent} is not a class");
    };

    let fields = self.cx.alloc(parent.fields.deref().clone());
    for (offset, key) in desc.fields.keys().enumerate() {
      let value = self.get_register(parts.offset(1 + offset));
      fields.insert(key, value);
    }
    let class = self.make_class(desc, Some(fields), Some(parent));

    self.acc = Value::object(class);

    Ok(())
  }

  fn op_make_list(&mut self, start: op::Register, count: op::Count) -> hebi::Result<()> {
    let list = List::with_capacity(count.value());
    for reg in start.iter(count, 1) {
      list.push(self.get_register(reg));
    }
    self.acc = Value::object(self.cx.alloc(list));
    Ok(())
  }

  fn op_make_list_empty(&mut self) -> hebi::Result<()> {
    self.acc = Value::object(self.cx.alloc(List::new()));
    Ok(())
  }

  fn op_make_table(&mut self, start: op::Register, count: op::Count) -> hebi::Result<()> {
    let table = Table::with_capacity(count.value());
    for reg in start.iter(count, 2) {
      let key = self.get_register(reg);
      let value = self.get_register(reg.offset(1));

      let Some(key) = key.clone().to_any().and_then(|v| v.cast::<String>().ok()) else {
        hebi::fail!( "`{key}` is not a string");
      };

      table.insert(key, value);
    }
    self.acc = Value::object(self.cx.alloc(table));
    Ok(())
  }

  fn op_make_table_empty(&mut self) -> hebi::Result<()> {
    self.acc = Value::object(self.cx.alloc(Table::new()));
    Ok(())
  }

  fn op_jump(&mut self, offset: op::Offset) -> hebi::Result<op::Offset> {
    Ok(offset)
  }

  fn op_jump_const(&mut self, idx: op::Constant) -> hebi::Result<op::Offset> {
    let offset = self.get_constant(idx).as_offset().cloned();
    debug_assert!(offset.is_some());
    let offset = unsafe { offset.unwrap_unchecked() };
    Ok(offset)
  }

  fn op_jump_loop(&mut self, offset: op::Offset) -> hebi::Result<op::Offset> {
    Ok(offset)
  }

  fn op_jump_if_false(&mut self, offset: op::Offset) -> hebi::Result<super::dispatch::Jump> {
    match is_truthy(take(&mut self.acc)) {
      true => Ok(super::dispatch::Jump::Skip),
      false => Ok(super::dispatch::Jump::Move(offset)),
    }
  }

  fn op_jump_if_false_const(&mut self, idx: op::Constant) -> hebi::Result<super::dispatch::Jump> {
    let offset = self.get_constant(idx).as_offset().cloned();
    debug_assert!(offset.is_some());
    let offset = unsafe { offset.unwrap_unchecked() };

    match is_truthy(take(&mut self.acc)) {
      true => Ok(super::dispatch::Jump::Move(offset)),
      false => Ok(super::dispatch::Jump::Skip),
    }
  }

  fn op_add(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_sub(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_mul(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_div(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_rem(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_pow(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_inv(&mut self) -> hebi::Result<()> {
    todo!()
  }

  fn op_not(&mut self) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_eq(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_ne(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_gt(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_ge(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_lt(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_le(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_cmp_type(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_contains(&mut self, lhs: op::Register) -> hebi::Result<()> {
    todo!()
  }

  fn op_is_none(&mut self) -> hebi::Result<()> {
    self.acc = Value::bool(self.acc.is_none());
    Ok(())
  }

  fn op_print(&mut self) -> hebi::Result<()> {
    // TODO: allow setting output writer
    println!("{}", self.acc);
    Ok(())
  }

  fn op_print_n(&mut self, start: op::Register, count: op::Count) -> hebi::Result<()> {
    debug_assert!(self.stack_base + start.index() + count.value() < self.stack.len());

    let start = start.index();
    let end = start + count.value();
    for index in start..end {
      let value = self.get_register(op::Register(index as u32));
      print!("{value}");
    }
    println!();

    Ok(())
  }

  fn op_call(
    &mut self,
    return_addr: usize,
    callee: op::Register,
    args: op::Count,
  ) -> hebi::Result<dispatch::Call> {
    let f = self.get_register(callee);
    let start = self.stack_base + callee.index() + 1;
    let (stack_base, num_args) = push_args!(self, f.clone(), range(start, start + args.value()));
    self.do_call(f, stack_base, num_args, Some(return_addr))
  }

  fn op_call0(&mut self, return_addr: usize) -> hebi::Result<dispatch::Call> {
    let f = take(&mut self.acc);
    let stack_base = self.stack.len();
    self.do_call(f, stack_base, 0, Some(return_addr))
  }

  fn op_import(&mut self, path: op::Constant, dst: op::Register) -> hebi::Result<()> {
    let path = self.get_constant_object::<String>(path);
    let module = self.load_module(path)?;
    self.set_register(dst, Value::object(module));

    Ok(())
  }

  fn op_return(&mut self) -> hebi::Result<dispatch::Return> {
    // return value is in the accumulator

    // pop frame
    let frame = self.call_frames.borrow_mut().pop().unwrap();

    // truncate stack
    self.stack.truncate(self.stack.len() - frame.frame_size);

    Ok(match self.call_frames.borrow().last() {
      Some(current_frame) => {
        self.stack_base -= current_frame.frame_size;
        if let Some(return_addr) = frame.return_addr {
          // restore pc
          self.pc = return_addr;
          dispatch::Return::LoadFrame(dispatch::LoadFrame {
            bytecode: current_frame.instructions,
            pc: self.pc,
          })
        } else {
          dispatch::Return::Yield
        }
      }
      None => dispatch::Return::Yield,
    })
  }

  fn op_yield(&mut self) -> hebi::Result<()> {
    todo!()
  }
}
