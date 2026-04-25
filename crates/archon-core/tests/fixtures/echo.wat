;; TASK-AGS-507: Minimal echo WASM module for plugin roundtrip tests.
;;
;; ABI:
;;   memory  — exported linear memory (1 page = 64 KiB)
;;   alloc(size: i32) -> i32  — bump-allocator returning a pointer
;;   pattern_execute(ptr: i32, len: i32) -> i64  — echoes input back
;;       return value: (out_ptr << 32) | out_len

(module
  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 1024))

  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $size)))
    (local.get $ptr)
  )

  (func (export "pattern_execute") (param $ptr i32) (param $len i32) (result i64)
    ;; Echo: return the same ptr/len packed into a single i64.
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
      (i64.extend_i32_u (local.get $len))
    )
  )
)
