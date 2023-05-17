use super::object::{AnyRef, ObjectRef};
use crate::value::Value;
use crate::{Bind, Context, Result, Unbind};

decl_ref! {
  struct Value
}

impl<'cx> ValueRef<'cx> {
  pub fn as_float(&self) -> Option<f64> {
    self.inner.clone().to_float()
  }

  pub fn as_int(&self) -> Option<i32> {
    self.inner.clone().to_int()
  }

  pub fn as_bool(&self) -> Option<bool> {
    self.inner.clone().to_bool()
  }

  pub fn as_none(&self) -> Option<()> {
    self.inner.clone().to_none()
  }

  pub fn as_object<T: ObjectRef<'cx>>(&self, cx: Context<'cx>) -> Option<T> {
    self.as_any().and_then(|v| AnyRef::cast(v, cx))
  }

  pub fn as_any(&self) -> Option<AnyRef<'cx>> {
    self.inner.clone().to_any().map(|v| {
      // SAFETY: `self` is already bound to 'cx
      unsafe { v.bind_raw::<'cx>() }
    })
  }
}

pub trait FromValue<'cx>: Sized {
  fn from_value(value: ValueRef<'cx>, cx: Context<'cx>) -> Result<Self>;
}

pub trait IntoValue<'cx>: Sized {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>>;
}

impl<'cx> IntoValue<'cx> for ValueRef<'cx> {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    let _ = cx;
    Ok(self)
  }
}

impl<'cx> FromValue<'cx> for ValueRef<'cx> {
  fn from_value(value: ValueRef<'cx>, cx: Context<'cx>) -> Result<Self> {
    let _ = cx;
    Ok(value)
  }
}

impl<'cx> IntoValue<'cx> for i32 {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    Ok(Value::int(self).bind(cx))
  }
}

impl<'cx> FromValue<'cx> for i32 {
  fn from_value(value: ValueRef<'cx>, cx: Context<'cx>) -> Result<Self> {
    let _ = cx;
    match value.as_int() {
      Some(value) => Ok(value),
      None => crate::fail!("value is not an int"),
    }
  }
}

impl<'cx> IntoValue<'cx> for f64 {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    Ok(Value::float(self).bind(cx))
  }
}

impl<'cx> FromValue<'cx> for f64 {
  fn from_value(value: ValueRef<'cx>, cx: Context<'cx>) -> Result<Self> {
    let _ = cx;
    match value.as_float() {
      Some(value) => Ok(value),
      None => crate::fail!("value is not an int"),
    }
  }
}

impl<'cx> IntoValue<'cx> for bool {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    Ok(Value::bool(self).bind(cx))
  }
}

impl<'cx> FromValue<'cx> for bool {
  fn from_value(value: ValueRef<'cx>, cx: Context<'cx>) -> Result<Self> {
    let _ = cx;
    match value.as_bool() {
      Some(value) => Ok(value),
      None => crate::fail!("value is not an int"),
    }
  }
}

impl<'cx> IntoValue<'cx> for () {
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    Ok(Value::none().bind(cx))
  }
}

impl<'cx, T> IntoValue<'cx> for Option<T>
where
  T: IntoValue<'cx>,
{
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    match self {
      Some(value) => value.into_value(cx),
      None => Ok(Value::none().bind(cx)),
    }
  }
}

impl<'cx, T> IntoValue<'cx> for Result<T>
where
  T: IntoValue<'cx>,
{
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    self?.into_value(cx)
  }
}

impl<'cx, T> IntoValue<'cx> for T
where
  T: ObjectRef<'cx>,
{
  fn into_value(self, cx: Context<'cx>) -> Result<ValueRef<'cx>> {
    Ok(Value::object(self.as_any(cx.clone()).unbind()).bind(cx))
  }
}
