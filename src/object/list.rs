use std::cell::RefCell;
use std::fmt::{Debug, Display};
use std::vec::Vec;

use super::builtin::BuiltinMethod;
use super::{Object, Ptr, Str};
use crate::util::{JoinIter, MAX_SAFE_INT, MIN_SAFE_INT};
use crate::value::Value;
use crate::{Result, Scope, Unbind};

#[derive(Default)]
pub struct List {
  data: RefCell<Vec<Value>>,
}

impl List {
  pub fn new() -> Self {
    Self::with_capacity(0)
  }

  pub fn with_capacity(n: usize) -> Self {
    Self {
      data: RefCell::new(Vec::with_capacity(n)),
    }
  }

  pub fn len(&self) -> usize {
    self.data.borrow().len()
  }

  pub fn is_empty(&self) -> bool {
    self.data.borrow().is_empty()
  }

  #[allow(dead_code)]
  pub fn get(&self, index: usize) -> Option<Value> {
    self.data.borrow().get(index).cloned()
  }

  pub fn push(&self, value: Value) {
    self.data.borrow_mut().push(value);
  }

  pub fn pop(&self) -> Option<Value> {
    self.data.borrow_mut().pop()
  }

  /// # Safety
  ///
  /// - `index` must be within the bounds of `self`
  pub unsafe fn get_unchecked(&self, index: usize) -> Value {
    debug_assert!(index < self.len(), "index {index} out of bounds");
    self.data.borrow().get_unchecked(index).clone()
  }

  #[must_use = "`set` returns false if index is out of bounds"]
  pub fn set(&self, index: usize, value: Value) -> bool {
    if let Some(slot) = self.data.borrow_mut().get_mut(index) {
      *slot = value;
      true
    } else {
      false
    }
  }

  /// # Safety
  ///
  /// - `index` must be within the bounds of `self`
  pub unsafe fn set_unchecked(&self, index: usize, value: Value) {
    debug_assert!(index < self.len(), "index {index} out of bounds");
    *self.data.borrow_mut().get_mut(index).unwrap_unchecked() = value;
  }

  pub fn iter(&self) -> Iter {
    Iter {
      list: self,
      index: 0,
    }
  }
}

#[derive(Clone)]
pub struct Iter<'a> {
  list: &'a List,
  index: usize,
}

impl<'a> Iterator for Iter<'a> {
  type Item = Value;

  fn next(&mut self) -> Option<Self::Item> {
    match self.list.data.borrow().get(self.index) {
      Some(value) => {
        self.index += 1;
        Some(value.clone())
      }
      None => None,
    }
  }
}

impl From<Vec<Value>> for List {
  fn from(values: Vec<Value>) -> Self {
    Self {
      data: RefCell::new(values),
    }
  }
}

impl Display for List {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "<list>")
  }
}

impl Debug for List {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_list().entries(&self.data.borrow()[..]).finish()
  }
}

fn list_len(this: Ptr<List>, _: Scope<'_>) -> Result<Value> {
  Ok(Value::int(this.len() as i32))
}

fn list_is_empty(this: Ptr<List>, _: Scope<'_>) -> Result<Value> {
  Ok(Value::bool(this.is_empty()))
}

fn list_get(this: Ptr<List>, scope: Scope<'_>) -> Result<Value> {
  let index = scope.param::<crate::Value>(0)?.unbind();
  let index = to_index(index, this.len())?;
  Ok(this.get(index).unwrap_or_else(Value::none))
}

fn list_set(this: Ptr<List>, scope: Scope<'_>) -> Result<Value> {
  let (index, value) = scope.params::<(crate::Value, crate::Value)>()?;
  let (index, value) = (index.unbind(), value.unbind());
  let len = this.len();
  let index = to_index(index, len)?;
  if !this.set(index, value) {
    fail!("index `{index}` out of bounds, len was `{len}`")
  }

  Ok(Value::none())
}

fn list_push(this: Ptr<List>, scope: Scope<'_>) -> Result<Value> {
  let value = scope.param::<crate::Value>(0)?.unbind();
  this.push(value);
  Ok(Value::none())
}

fn list_pop(this: Ptr<List>, _: Scope<'_>) -> Result<Value> {
  Ok(this.pop().unwrap_or_else(Value::none))
}

fn list_join(this: Ptr<List>, scope: Scope<'_>) -> Result<Value> {
  let sep = scope.param::<crate::Str>(0)?;
  Ok(Value::object(
    scope.alloc(Str::owned(this.iter().join(sep.as_str()))),
  ))
}

// TODO: list iter

impl Object for List {
  fn type_name(_: Ptr<Self>) -> &'static str {
    "List"
  }

  fn named_field(this: Ptr<Self>, scope: Scope<'_>, name: Ptr<super::Str>) -> Result<Value> {
    let method = match name.as_str() {
      "len" => builtin_method!(list_len),
      "is_empty" => builtin_method!(list_is_empty),
      "get" => builtin_method!(list_get),
      "set" => builtin_method!(list_set),
      "push" => builtin_method!(list_push),
      "pop" => builtin_method!(list_pop),
      "join" => builtin_method!(list_join),
      _ => fail!("`{this}` has no field `{name}`"),
    };

    Ok(Value::object(unsafe {
      scope.alloc(BuiltinMethod::new(Value::object(this), method))
    }))
  }

  fn named_field_opt(
    this: Ptr<Self>,
    scope: Scope<'_>,
    name: Ptr<super::Str>,
  ) -> Result<Option<Value>> {
    let method = match name.as_str() {
      "len" => builtin_method!(|this, _| { Ok(Value::int(this.len() as i32)) }),
      _ => return Ok(None),
    };

    Ok(Some(Value::object(unsafe {
      scope.alloc(BuiltinMethod::new(Value::object(this), method))
    })))
  }

  fn keyed_field(this: Ptr<Self>, _: crate::Scope<'_>, key: Value) -> Result<Value> {
    let len = this.len();
    let index = to_index(key.clone(), len)?;
    let value = this
      .get(index)
      .ok_or_else(|| error!("index `{key}` out of bounds, len was `{len}`"))?;
    Ok(value)
  }

  fn keyed_field_opt(this: Ptr<Self>, _: Scope<'_>, key: Value) -> Result<Option<Value>> {
    let len = this.len();
    let index = to_index(key, len)?;
    Ok(this.get(index))
  }

  fn set_keyed_field(this: Ptr<Self>, _: crate::Scope<'_>, key: Value, value: Value) -> Result<()> {
    let len = this.len();
    let index = to_index(key.clone(), len)?;
    if !this.set(index, value) {
      fail!("index `{key}` out of bounds, len was `{len}`");
    };
    Ok(())
  }
}

fn to_index(index: Value, len: usize) -> Result<usize> {
  if index.is_int() {
    let index = unsafe { index.to_int().unwrap_unchecked() };
    let index = if index.is_negative() {
      len - ((-index) as usize)
    } else {
      index as usize
    };
    return Ok(index);
  } else if index.is_float() {
    let index = unsafe { index.clone().to_float().unwrap_unchecked() };
    if index.is_finite() && index.fract() == 0.0 && (MIN_SAFE_INT..=MAX_SAFE_INT).contains(&index) {
      return Ok(index as usize);
    }
  };

  fail!("`{index}` is not a valid index")
}

declare_object_type!(List);
