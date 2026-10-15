;; ゲスト(仮想コンピュータ上で動くプログラム)。WAT形式のままWeaveModule.loadに渡せます。
;; Rust/C等でビルドした .wasm を渡すのも同じ方法です。
(module
  ;; ホスト(Java側)が提供する関数の宣言
  (import "sys" "print" (func $print (param i32 i32)))          ;; (ptr, len) を画面に出力
  (import "sys" "add"   (func $add   (param i32 i32) (result i32)))

  (memory (export "memory") 1)
  (data (i32.const 0) "Hello from the guest! tick=")

  ;; main(n): n回ループして、毎回メッセージを出力する。
  ;; 戻り値は「ホストのaddで合計した値」(0+1+...+(n-1))。
  (func (export "main") (param $n i32) (result i32)
    (local $i i32)
    (local $sum i32)
    (loop $l
      (call $print (i32.const 0) (i32.const 27))
      ;; ホスト関数の呼び出しの実演
      (local.set $sum (call $add (local.get $sum) (local.get $i)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (local.get $n))))
    (local.get $sum)))
