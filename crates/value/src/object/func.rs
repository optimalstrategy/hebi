use beef::lean::Cow;

use crate::Value;

#[derive(Clone, Debug)]
pub struct Func {
  pub(super) name: Cow<'static, str>,
  pub(super) frame_size: u32,
  pub(super) code: Vec<u8>,
  pub(super) const_pool: Vec<Value>,
  pub(super) params: Params,
}

#[derive(Clone, Debug)]
pub struct ClosureDescriptor {
  pub func: Func,
  pub num_captures: u32,
}

#[derive(Clone, Debug)]
pub struct Closure {
  pub(super) descriptor: Value,
  pub(super) captures: Vec<Value>,
}

impl Closure {
  /// Create a new closure.
  ///
  /// # Panics
  /// If `func.is_closure_descriptor()` is not `true`.
  pub fn new(descriptor: Value) -> Self {
    // TODO: can this be encoded via the type system?
    assert!(
      descriptor.is_closure_descriptor(),
      "closure may only be constructed from a closure descriptor"
    );

    let captures = {
      let descriptor = descriptor.as_closure_descriptor().unwrap();
      let mut v = Vec::with_capacity(descriptor.num_captures as usize);
      for _ in 0..descriptor.num_captures {
        v.push(Value::none());
      }
      v
    };

    Self {
      descriptor,
      captures,
    }
  }
}

#[derive(Clone, Debug)]
pub struct Params {
  pub min: u32,
  pub max: Option<u32>,
  pub kw: indexmap::IndexSet<String>,
}

impl Func {
  pub fn new(
    name: Cow<'static, str>,
    frame_size: u32,
    code: Vec<u8>,
    const_pool: Vec<Value>,
    params: Params,
  ) -> Self {
    Self {
      name,
      frame_size,
      code,
      const_pool,
      params,
    }
  }

  pub fn name(&self) -> &str {
    self.name.as_ref()
  }

  pub fn frame_size(&self) -> u32 {
    self.frame_size
  }

  pub fn code(&self) -> &[u8] {
    &self.code
  }

  pub fn const_pool(&self) -> &[Value] {
    &self.const_pool
  }

  pub fn params(&self) -> &Params {
    &self.params
  }

  pub fn disassemble<F, D>(&self, disassemble_instruction: F, print_bytes: bool) -> String
  where
    F: Fn(&[u8], usize) -> (usize, D),
    D: std::fmt::Display,
  {
    let mut out = String::new();

    for v in self.const_pool.iter() {
      if let Some(func) = v.as_func() {
        func.disassemble_inner(&disassemble_instruction, &mut out, print_bytes);
        out += "\n";
      }
    }

    self.disassemble_inner(&disassemble_instruction, &mut out, print_bytes);
    out
  }

  fn disassemble_inner<F, D>(&self, disassemble_instruction: F, f: &mut String, print_bytes: bool)
  where
    F: Fn(&[u8], usize) -> (usize, D),
    D: std::fmt::Display,
  {
    use std::fmt::Write;

    // name
    writeln!(f, "function <{}>:", self.name).unwrap();
    writeln!(f, "  frame_size: {}", self.frame_size).unwrap();
    writeln!(f, "  length: {}", self.code.len()).unwrap();

    // constants
    if self.const_pool.is_empty() {
      writeln!(f, "  const: <empty>").unwrap();
    } else {
      writeln!(f, "  const (length={}):", self.const_pool.len()).unwrap();
      for (i, value) in self.const_pool.iter().enumerate() {
        writeln!(f, "    {i}: {value}").unwrap();
      }
    }

    // bytecode
    writeln!(f, "  code:").unwrap();
    let offset_align = self.code.len().to_string().len();
    let mut pc = 0;
    while pc < self.code.len() {
      let (size, instr) = disassemble_instruction(&self.code[..], pc);

      let bytes = {
        let mut out = String::new();
        if print_bytes {
          for byte in self.code[pc..pc + size].iter() {
            write!(&mut out, "{byte:02x} ").unwrap();
          }
          if size < 6 {
            for _ in 0..(6 - size) {
              write!(&mut out, "   ").unwrap();
            }
          }
        }
        out
      };

      writeln!(f, "    {pc:offset_align$} | {bytes}{instr}").unwrap();

      pc += size;
    }
  }
}
