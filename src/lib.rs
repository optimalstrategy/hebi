#![allow(clippy::wrong_self_convention)]

mod conv;
mod ctx;
mod emit;
mod isolate;
mod op;
mod util;
mod value;

/*

TODO: carefully design the public API
- Value
  - constructors
  - as_*
- Isolate
  - call
  - ?
*/

// TODO: everything that allocates should go through the context,
// eventually the context `alloc` method will use a faster allocator
// together with a garbage collector to make it worth the effort

use std::cell::{Ref, RefCell};
use std::fmt::{Debug, Display};

use ctx::Context;
use isolate::{Isolate, Stdout};
use value::Value as CoreValue;

pub type Result<T, E = RuntimeError> = std::result::Result<T, E>;

pub use conv::{FromHebi, IntoHebi, Value};
pub use value::object::module::ModuleLoader;
pub use value::object::RuntimeError;

pub struct Hebi {
  isolate: RefCell<Isolate>,
}

// # Safety:
// Internally, the VM uses reference counting using the `Rc` type.
// Normally, it would be unsound to implement Send for something that
// uses `Rc`, but in this case, the user can *never* obtain an internal
// `Rc` (or equivalent). This means they can never clone that `Rc`, and
// then move their `Hebi` instance to another thread, causing a data race
// between the user's clone of the `Rc` and Hebi's clone.
//
// This is enforced by via the `conv::Value<'a>` type, which borrows from
// `Hebi`, meaning that `Hebi` may not be moved (potentially to another thread)
// while that value is borrowed.
unsafe impl Send for Hebi {}

impl Hebi {
  pub fn new() -> Self {
    Self::builder().build()
  }

  pub fn with_io(io: impl Stdout) -> Self {
    Self::builder().with_io(io).build()
  }

  pub fn check(&self, src: &str) -> Result<(), Vec<syntax::Error>> {
    syntax::parse(src)?;
    Ok(())
  }

  pub fn eval<'a, T: FromHebi<'a>>(&'a self, src: &str) -> Result<T, EvalError> {
    let ctx = self.isolate.borrow().ctx();
    let module = syntax::parse(src)?;
    let module = emit::emit(ctx.clone(), "code", &module, true).unwrap();
    let module = module.instance(&ctx, None);
    let result = self
      .isolate
      .borrow_mut()
      .call(module.root().into(), &[], CoreValue::none())?;
    let result = Value::bind(result);
    Ok(T::from_hebi(self, result)?)
  }

  pub fn io<T: 'static>(&self) -> Option<Ref<'_, T>> {
    match Ref::filter_map(self.isolate.borrow(), |isolate| {
      isolate.io().as_any().downcast_ref()
    }) {
      Ok(v) => Some(v),
      _ => None,
    }
  }
}

pub struct HebiBuilder {
  stdout: Option<Box<dyn Stdout>>,
  module_loader: Option<Box<dyn ModuleLoader>>,
}

impl Hebi {
  pub fn builder() -> HebiBuilder {
    HebiBuilder {
      stdout: None,
      module_loader: None,
    }
  }
}

impl HebiBuilder {
  pub fn with_io<T: Stdout + 'static>(mut self, stdout: T) -> Self {
    let _ = self.stdout.replace(Box::new(stdout));
    self
  }

  pub fn with_module_loader<T: ModuleLoader + 'static>(mut self, loader: T) -> Self {
    let _ = self.module_loader.replace(Box::new(loader));
    self
  }

  pub fn build(mut self) -> Hebi {
    let ctx = Context::new();
    let stdout = self
      .stdout
      .take()
      .unwrap_or_else(|| Box::new(std::io::stdout()));
    let module_loader = self
      .module_loader
      .take()
      .unwrap_or_else(|| Box::new(NoopModuleLoader));
    let isolate = Isolate::new(ctx, stdout, module_loader);

    Hebi {
      isolate: RefCell::new(isolate),
    }
  }
}

impl Default for Hebi {
  fn default() -> Self {
    Self::new()
  }
}

/// The noop module loader refuses to load any modules.
pub struct NoopModuleLoader;
#[derive(Debug)]
pub struct ModuleLoadError {
  pub path: String,
}
impl Display for ModuleLoadError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "could not load module `{}`", self.path)
  }
}
impl std::error::Error for ModuleLoadError {}
impl ModuleLoader for NoopModuleLoader {
  fn load(
    &mut self,
    path: &[String],
  ) -> std::result::Result<&str, Box<dyn std::error::Error + 'static>> {
    Err(Box::new(ModuleLoadError {
      path: format!("could not load module `{}`", path.join(".")),
    }))
  }
}

pub enum EvalError {
  Parse(Vec<syntax::Error>),
  Runtime(RuntimeError),
}

impl From<Vec<syntax::Error>> for EvalError {
  fn from(value: Vec<syntax::Error>) -> Self {
    EvalError::Parse(value)
  }
}
impl From<RuntimeError> for EvalError {
  fn from(value: RuntimeError) -> Self {
    EvalError::Runtime(value)
  }
}

impl Debug for EvalError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Parse(e) => f
        .debug_tuple("Parse")
        .field(&e.iter().map(|e| e.message.to_string()).collect::<Vec<_>>())
        .finish(),
      Self::Runtime(e) => f.debug_tuple("Runtime").field(&e).finish(),
    }
  }
}
impl Display for EvalError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use util::JoinIter;
    match self {
      EvalError::Parse(v) => write!(f, "syntax errors: {}", v.iter().join("; ")),
      EvalError::Runtime(v) => write!(f, "error: {v}"),
    }
  }
}
impl std::error::Error for EvalError {}

#[cfg(test)]
mod tests;
