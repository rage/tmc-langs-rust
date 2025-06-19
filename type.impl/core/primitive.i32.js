(function() {
    var type_impls = Object.fromEntries([["jni_sys",[]],["libc",[]],["linux_raw_sys",[]],["unsafe_libyaml",[]]]);
    if (window.register_type_impls) {
        window.register_type_impls(type_impls);
    } else {
        window.pending_type_impls = type_impls;
    }
})()
//{"start":55,"fragment_lengths":[14,12,21,22]}