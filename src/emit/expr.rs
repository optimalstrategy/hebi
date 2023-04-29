use super::*;
use crate::value::constant::NonNaNFloat;

impl<'cx, 'src> State<'cx, 'src> {
  pub fn emit_expr(&mut self, expr: &'src ast::Expr<'src>) {
    match &**expr {
      ast::ExprKind::Literal(v) => self.emit_literal_expr(v, expr.span),
      ast::ExprKind::Binary(v) => self.emit_binary_expr(v, expr.span),
      ast::ExprKind::Unary(v) => self.emit_unary_expr(v, expr.span),
      ast::ExprKind::GetVar(v) => self.emit_get_var_expr(v, expr.span),
      ast::ExprKind::SetVar(v) => self.emit_set_var_expr(v, expr.span),
      ast::ExprKind::GetField(v) => self.emit_get_field_expr(v, expr.span),
      ast::ExprKind::SetField(v) => self.emit_set_field_expr(v, expr.span),
      ast::ExprKind::GetIndex(v) => self.emit_get_index_expr(v, expr.span),
      ast::ExprKind::SetIndex(v) => self.emit_set_index_expr(v, expr.span),
      ast::ExprKind::Call(v) => self.emit_call_expr(v, expr.span),
      ast::ExprKind::GetSelf => self.emit_get_self_expr(expr.span),
      ast::ExprKind::GetSuper => self.emit_get_super_expr(expr.span),
    }
  }

  fn emit_literal_expr(&mut self, expr: &'src ast::Literal<'src>, span: Span) {
    match expr {
      ast::Literal::None => self.builder().emit(LoadNone, span),
      ast::Literal::Int(v) => self.builder().emit(LoadSmi { value: op::Smi(*v) }, span),
      ast::Literal::Float(v) => {
        // float is 4 bits so cannot be stored inline,
        // but it is interned
        let num = self.constant_value(NonNaNFloat::try_from(*v).unwrap());
        self.builder().emit(LoadConst { index: num }, span);
      }
      ast::Literal::Bool(v) => match v {
        true => self.builder().emit(LoadTrue, span),
        false => self.builder().emit(LoadFalse, span),
      },
      ast::Literal::String(v) => {
        // `const_` interns the string
        let str = self.constant_name(v);
        self.builder().emit(LoadConst { index: str }, span);
      }
      ast::Literal::List(list) => {
        if list.is_empty() {
          self.builder().emit(MakeListEmpty, span);
          return;
        }

        let registers = (0..list.len())
          .map(|_| self.alloc_register())
          .collect::<Vec<_>>();
        for (value, register) in list.iter().zip(registers.iter()) {
          self.emit_expr(value);
          self.builder().emit(
            Store {
              register: register.access(),
            },
            value.span,
          );
        }
        self.builder().emit(
          MakeList {
            start: registers[0].access(),
            count: op::Count(list.len() as u32),
          },
          span,
        );
        for register in registers.iter().rev() {
          register.access();
        }
      }
      ast::Literal::Table(table) => {
        if table.is_empty() {
          self.builder().emit(MakeTableEmpty, span);
          return;
        }

        // TODO: from descriptor
        let registers = (0..table.len())
          .map(|_| (self.alloc_register(), self.alloc_register()))
          .collect::<Vec<_>>();
        for ((key, value), (key_register, value_register)) in table.iter().zip(registers.iter()) {
          self.emit_expr(key);
          self.builder().emit(
            Store {
              register: key_register.access(),
            },
            key.span,
          );
          self.emit_expr(value);
          self.builder().emit(
            Store {
              register: value_register.access(),
            },
            value.span,
          );
        }
        self.builder().emit(
          MakeTable {
            start: registers[0].0.access(),
            count: op::Count(table.len() as u32),
          },
          span,
        );
        for (key, value) in registers.iter().rev() {
          value.access();
          key.access();
        }
      }
    }
  }

  fn emit_binary_expr(&mut self, expr: &'src ast::Binary<'src>, span: Span) {
    // binary expressions store lhs in a register,
    // and rhs in the accumulator

    match expr.op {
      ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Maybe => {
        return self.emit_logical_expr(expr, span)
      }
      _ => {}
    }

    let lhs = self.alloc_register();
    self.emit_expr(&expr.left);
    self.builder().emit(
      Store {
        register: lhs.access(),
      },
      expr.left.span,
    );
    self.emit_expr(&expr.right);

    let lhs = lhs.access();
    match expr.op {
      ast::BinaryOp::Add => self.builder().emit(Add { lhs }, span),
      ast::BinaryOp::Sub => self.builder().emit(Sub { lhs }, span),
      ast::BinaryOp::Div => self.builder().emit(Div { lhs }, span),
      ast::BinaryOp::Mul => self.builder().emit(Mul { lhs }, span),
      ast::BinaryOp::Rem => self.builder().emit(Rem { lhs }, span),
      ast::BinaryOp::Pow => self.builder().emit(Pow { lhs }, span),
      ast::BinaryOp::Eq => self.builder().emit(CmpEq { lhs }, span),
      ast::BinaryOp::Neq => self.builder().emit(CmpNe { lhs }, span),
      ast::BinaryOp::More => self.builder().emit(CmpGt { lhs }, span),
      ast::BinaryOp::MoreEq => self.builder().emit(CmpGe { lhs }, span),
      ast::BinaryOp::Less => self.builder().emit(CmpLt { lhs }, span),
      ast::BinaryOp::LessEq => self.builder().emit(CmpLe { lhs }, span),
      ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Maybe => unreachable!(),
    }
  }

  fn emit_logical_expr(&mut self, expr: &'src ast::Binary<'src>, span: Span) {
    match expr.op {
      ast::BinaryOp::And => {
        /*
          <left> && <right>
          v = <left>
          if v:
            v = <right>
        */
        let end = self.builder().label("end");
        self.emit_expr(&expr.left);
        self.builder().emit_jump_if_false(&end, span);
        self.emit_expr(&expr.right);
        self.builder().bind_label(end);
      }
      ast::BinaryOp::Or => {
        /*
          <left> || <right>
          v = <left>
          if !v:
            v = <right>
        */
        let rhs = self.builder().label("rhs");
        let end = self.builder().label("end");
        self.emit_expr(&expr.left);
        self.builder().emit_jump_if_false(&rhs, span);
        self.builder().emit_jump(&end, span);
        self.builder().bind_label(rhs);
        self.emit_expr(&expr.right);
        self.builder().bind_label(end);
      }
      ast::BinaryOp::Maybe => {
        /*
          <left> ?? <right>
          v = <left>
          if v is none:
            v = <right>
        */
        let use_lhs = self.builder().label("lhs");
        let end = self.builder().label("end");
        let lhs = self.alloc_register();
        self.emit_expr(&expr.left);
        self.builder().emit(
          Store {
            register: lhs.access(),
          },
          expr.left.span,
        );
        self.builder().emit(IsNone, span);
        self.builder().emit_jump_if_false(&use_lhs, span);
        self.emit_expr(&expr.right);
        self.builder().emit_jump(&end, span);
        self.builder().bind_label(use_lhs);
        self.builder().emit(
          Load {
            register: lhs.access(),
          },
          span,
        );
        self.builder().bind_label(end);
      }
      _ => unreachable!("not a logical expr: {:?}", expr.op),
    }
  }

  fn emit_unary_expr(&mut self, expr: &'src ast::Unary<'src>, span: Span) {
    // unary expressions only use the accumulator

    if matches!(expr.op, ast::UnaryOp::Opt) {
      return self.emit_opt_expr(expr);
    }

    self.emit_expr(&expr.right);

    match expr.op {
      ast::UnaryOp::Plus => {}
      ast::UnaryOp::Minus => self.builder().emit(Inv, span),
      ast::UnaryOp::Not => self.builder().emit(Not, span),
      ast::UnaryOp::Opt => unreachable!(),
    }
  }

  fn emit_opt_expr(&mut self, expr: &'src ast::Unary<'src>) {
    assert!(matches!(expr.op, ast::UnaryOp::Opt));

    // - emit_call_expr <- with receiver, `CallMethodOpt` or similar

    let prev = std::mem::replace(&mut self.current_function().is_in_opt_expr, true);
    self.emit_expr(&expr.right);
    let _ = std::mem::replace(&mut self.current_function().is_in_opt_expr, prev);
  }

  fn emit_get_var_expr(&mut self, expr: &'src ast::GetVar<'src>, span: Span) {
    match self.resolve_var(expr.name.lexeme()) {
      Get::Local(reg) => self.builder().emit(
        Load {
          register: reg.access(),
        },
        span,
      ),
      Get::Upvalue(index) => self.builder().emit(LoadUpvalue { index }, span),
      Get::ModuleVar(index) => self.builder().emit(LoadModuleVar { index }, span),
      Get::Global => {
        let name = self.constant_name(&expr.name);
        self.builder().emit(LoadGlobal { name }, span)
      }
    }
  }

  fn emit_set_var_expr(&mut self, expr: &'src ast::SetVar<'src>, span: Span) {
    self.emit_expr(&expr.value);
    match self.resolve_var(expr.target.name.lexeme()) {
      Get::Local(reg) => self.builder().emit(
        Store {
          register: reg.access(),
        },
        span,
      ),
      Get::Upvalue(index) => self.builder().emit(StoreUpvalue { index }, span),
      Get::ModuleVar(index) => self.builder().emit(StoreModuleVar { index }, span),
      Get::Global => {
        let name = self.constant_name(&expr.target.name);
        self.builder().emit(StoreGlobal { name }, span);
      }
    }
  }

  fn emit_get_field_expr(&mut self, expr: &'src ast::GetField<'src>, span: Span) {
    let name = self.constant_name(&expr.name);
    self.emit_expr(&expr.target);
    if self.current_function().is_in_opt_expr {
      self.builder().emit(LoadFieldOpt { name }, span);
    } else {
      self.builder().emit(LoadField { name }, span);
    }
  }

  fn emit_set_field_expr(&mut self, expr: &'src ast::SetField<'src>, span: Span) {
    let object = self.alloc_register();
    let name = self.constant_name(&expr.target.name);
    self.emit_expr(&expr.target.target);
    self.builder().emit(
      Store {
        register: object.access(),
      },
      expr.target.target.span,
    );
    self.emit_expr(&expr.value);
    self.builder().emit(
      StoreField {
        object: object.access(),
        name,
      },
      span,
    );
  }

  fn emit_get_index_expr(&mut self, expr: &'src ast::GetIndex<'src>, span: Span) {
    let object = self.alloc_register();
    self.emit_expr(&expr.target);
    self.builder().emit(
      Store {
        register: object.access(),
      },
      expr.target.span,
    );
    self.emit_expr(&expr.key);
    if self.current_function().is_in_opt_expr {
      self.builder().emit(
        LoadIndexOpt {
          object: object.access(),
        },
        span,
      );
    } else {
      self.builder().emit(
        LoadIndex {
          object: object.access(),
        },
        span,
      );
    }
  }

  fn emit_set_index_expr(&mut self, expr: &'src ast::SetIndex<'src>, span: Span) {
    let object = self.alloc_register();
    let key = self.alloc_register();
    self.emit_expr(&expr.target.target);
    self.builder().emit(
      Store {
        register: object.access(),
      },
      expr.target.target.span,
    );
    self.emit_expr(&expr.target.key);
    self.builder().emit(
      Store {
        register: key.access(),
      },
      expr.target.key.span,
    );
    self.emit_expr(&expr.value);
    self.builder().emit(
      StoreIndex {
        object: object.access(),
        key: key.access(),
      },
      span,
    );
  }

  fn emit_call_expr(&mut self, expr: &'src ast::Call<'src>, span: Span) {
    // emit callee
    // emit args
    // emit op

    let callee = self.alloc_register();
    let args = (0..expr.args.len())
      .map(|_| self.alloc_register())
      .collect::<Vec<_>>();

    self.emit_expr(&expr.target);
    if args.is_empty() {
      self.builder().emit(Call0, span);
    } else {
      self.builder().emit(
        Store {
          register: callee.access(),
        },
        expr.target.span,
      );
      for (value, register) in expr.args.iter().zip(args.iter()) {
        self.emit_expr(value);
        self.builder().emit(
          Store {
            register: register.access(),
          },
          value.span,
        );
      }
      for arg in args.iter().rev() {
        arg.access();
      }
      self.builder().emit(
        Call {
          function: callee.access(),
          args: op::Count(args.len() as u32),
        },
        span,
      );
    }
  }

  fn emit_get_self_expr(&mut self, span: Span) {
    self.builder().emit(LoadSelf, span);
  }

  fn emit_get_super_expr(&mut self, span: Span) {
    self.builder().emit(LoadSuper, span);
  }
}
