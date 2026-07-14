use crate::common;

#[test]
fn matches_cpython_marshal_for_try_ifexp_attribute_call_none_condition_owner() {
    let source = r"class Pin:
    def remove_from(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = Pin.get_from(obj)
            if pin is not None:
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def global_call_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = get_pin(obj)
            if pin is not None:
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def no_ifexp_control(self, obj):
        try:
            pin_name = default_name
            pin = Pin.get_from(obj)
            if pin is not None:
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def name_test_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = Pin.get_from(obj)
            if pin:
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def call_test_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = Pin.get_from(obj)
            if accepted(pin):
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def body_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = get_pin(obj)
            if pin is not None:
                value = pin
        except AttributeError:
            recover(obj)

    def else_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = get_pin(obj)
            if pin is not None:
                delattr(obj, pin_name)
            else:
                recover(obj)
        except AttributeError:
            recover(obj)

    def named_handler_control(self, obj):
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = get_pin(obj)
            if pin is not None:
                delattr(obj, pin_name)
        except AttributeError as error:
            recover(error)

    def nested_with_control(self, obj, manager):
        try:
            with manager:
                pin_name = proxy_name if is_wrapped(obj) else default_name
                pin = get_pin(obj)
                if pin is not None:
                    delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

    def generator_control(self, obj):
        yield obj
        try:
            pin_name = proxy_name if is_wrapped(obj) else default_name
            pin = Pin.get_from(obj)
            if pin is not None:
                delattr(obj, pin_name)
        except AttributeError:
            recover(obj)

def function_positive(obj):
    try:
        pin_name = proxy_name if is_wrapped(obj) else default_name
        pin = Pin.get_from(obj)
        if pin is not None:
            delattr(obj, pin_name)
    except AttributeError:
        recover(obj)
";
    common::assert_matches_cpython_marshal(
        source,
        "try_ifexp_attribute_call_none_condition_owner.py",
    );
}
