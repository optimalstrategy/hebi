// TODO: cache current call frame in a field (Option<T>),
// so that it's one less indirection access

macro_rules! current_call_frame {
  ($self:ident) => {{
    debug_assert!(
      !$self.call_frames.borrow().is_empty(),
      "call frame stack is empty"
    );
    ::std::cell::Ref::map($self.call_frames.borrow(), |frames| unsafe {
      frames.last().unwrap_unchecked()
    })
  }};
}

macro_rules! current_call_frame_mut {
  ($self:ident) => {{
    debug_assert!(
      !$self.call_frames.borrow().is_empty(),
      "call frame stack is empty"
    );
    ::std::cell::RefMut::map($self.call_frames.borrow_mut(), |frames| unsafe {
      frames.last_mut().unwrap_unchecked()
    })
  }};
}

macro_rules! push_args {
  ($self:ident, $reserved:expr, range($start:expr, $end:expr)) => {{
    let reserved = $reserved;
    let start = $start;
    let end = $end;
    let stack_base = $self.stack.len();
    let num_args = end - start;
    $self.stack.push(reserved);
    $self.stack.extend_from_within(start..end);
    (stack_base, num_args)
  }};
  ($self:ident, $reserved:expr, $args:expr) => {{
    let reserved = $reserved;
    let args = $args;
    let stack_base = $self.stack.len();
    let num_args = args.len();
    $self.stack.push(reserved);
    $self.stack.extend_from_slice(args);
    (stack_base, num_args)
  }};
  ($self:ident, $args:expr) => {{
    let args = $args;
    let stack_base = $self.stack.len();
    let num_args = args.len();
    $self.stack.extend_from_slice(args);
    (stack_base, num_args)
  }};
}
