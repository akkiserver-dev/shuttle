package example;

import io.github.stefanrichterhuber.wasmtimejavang.WasmtimeEngine;
import io.github.stefanrichterhuber.wasmtimejavang.WeaveInstance;
import io.github.stefanrichterhuber.wasmtimejavang.WeaveModule;
import io.github.stefanrichterhuber.wasmtimejavang.WeaveResult;

import java.io.IOException;
import java.io.InputStream;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * OpenComputers / RetroComputers のようなMinecraft MOD向けの使い方を示す完全なサンプル。
 *
 * <p>やること:
 * <ol>
 *   <li>WebAssemblyプログラム(ゲスト)を「Weave変換」してロードする</li>
 *   <li>ホスト関数(print / add)をゲストに提供する</li>
 *   <li>1 tickあたりの実行量(予算)を決めて、サーバーtickごとに少しずつ実行する</li>
 *   <li>ワールド保存を想定して、実行途中の状態を丸ごとバイト列(スナップショット)に保存する</li>
 *   <li>ワールド読み込みを想定して、スナップショットから<b>続きから</b>再開する</li>
 * </ol>
 *
 * <p>Weaveを使う最大の利点は、ゲストが途中(ループの真っ最中など)でも安全に停止でき、
 * その状態をバイト列にして保存・復元できることです。通常のwasmでは不可能です。
 */
public final class VirtualComputerExample {

    /** 1 tickで実行するポーリング数(= 実行予算)。小さいほど1 tickあたりの負荷が軽くなる。 */
    private static final long BUDGET_PER_TICK = 2;

    public static void main(String[] args) throws Exception {
        byte[] wasm = readResource("/guest.wat");

        // エンジンはJVM内で共有して使い回せる。モジュールも同様(インスタンス作成は軽い)。
        try (WasmtimeEngine engine = new WasmtimeEngine();
                // pollPeriod=1: ループの周回ごとに「止めてよいか」を確認する。
                // 大きくすると速くなるが、止まれるまでの粒度が粗くなる(0以下でデフォルト値)。
                WeaveModule module = WeaveModule.load(engine, wasm, 1, 0)) {

            System.out.println("== ゲストのエントリポイント ==");
            module.entries().forEach((name, sig) -> System.out.println("  " + name + " : " + sig));

            // 画面の行番号。ホスト関数から共有する状態(スナップショットには含まれない点に注意)。
            AtomicInteger screenLine = new AtomicInteger();

            // ---------------------------------------------------------------
            // 1) 新規起動して、数tickだけ動かす
            // ---------------------------------------------------------------
            byte[] snapshot;
            try (WeaveInstance computer = newBuilder(module, screenLine).build()) {
                System.out.println("== 起動 (tick 1) ==");
                computer.unwindAfter(BUDGET_PER_TICK);
                WeaveResult r = computer.call("main", 12);
                int tick = 1;

                // 3 tick目まで進める。途中で予算切れになるとisUnwound()==trueで戻ってくる
                while (r.isUnwound() && tick < 3) {
                    tick++;
                    System.out.println("-- tick " + tick);
                    computer.unwindAfter(BUDGET_PER_TICK);
                    r = computer.resume();
                }

                // ---------------------------------------------------------------
                // 2) ワールド保存を想定: 停止中のゲスト状態をバイト列にする
                //    (gzip圧縮済み。NBTのByteArrayTagにそのまま入れられる)
                //    ※ snapshot() は停止(unwound)中にだけ呼べる
                // ---------------------------------------------------------------
                if (!r.isUnwound()) {
                    System.out.println("(予算が大きすぎて保存前に終了しました)");
                    return;
                }
                snapshot = computer.snapshot();
                System.out.println("== スナップショット保存: " + snapshot.length + " bytes ==");
            } // ← ここでcloseされる。「サーバー停止」に相当

            // ---------------------------------------------------------------
            // 3) ワールド読み込みを想定: 同じモジュールから復元して続きを実行
            //    ホスト関数は復元時にも同じ定義を渡す必要がある。
            // ---------------------------------------------------------------
            try (WeaveInstance restored = newBuilder(module, screenLine).restore(snapshot)) {
                System.out.println("== 復元して再開 (途中から続く) ==");
                int tick = 4;
                WeaveResult r;
                do {
                    System.out.println("-- tick " + tick++);
                    restored.unwindAfter(BUDGET_PER_TICK);
                    r = restored.resume();
                } while (r.isUnwound());
                System.out.println("== 完了。mainの戻り値(0..11の合計=66) = " + r.intValue() + " ==");
            }

            // ---------------------------------------------------------------
            // 補足: 別スレッドからの強制停止(電源OFFなど)は Cancellation を使う。
            //   try (WeaveInstance.Cancellation c = computer.cancellation()) {
            //       c.cancel(); // 次のポーリングでゲストが中断される
            //   }
            // ホスト関数の中から自分で止めたい場合(sys.yield など)は
            //   caller.requestUnwind();
            // を呼ぶと、次の安全な地点でcall/resumeが「unwound」で戻る。
            // ---------------------------------------------------------------
        }
    }

    /** ホスト関数を定義済みのBuilderを作る。新規起動・復元の両方で使う。 */
    private static WeaveInstance.Builder newBuilder(WeaveModule module, AtomicInteger screenLine) {
        return module.instance()
                // シグネチャは "引数>戻り値"。i=i32, l=i64, f=f32, d=f64。
                // sys.print(ptr, len): ゲストのメモリから文字列を読んで表示する
                .define("sys", "print", "ii>", (caller, a) -> {
                    String text = caller.readString(a[0], (int) a[1]);
                    System.out.println("  [画面 " + screenLine.incrementAndGet() + "] " + text);
                    return null; // 戻り値なし
                })
                // sys.add(a, b) -> a + b: 引数・戻り値は生のビット列(long)。i32は下位32bitを使う。
                // f32/f64は WeaveCaller.asFloat/fromFloat, asDouble/fromDouble で変換する。
                .define("sys", "add", "ii>i", (caller, a) -> new long[] { (int) a[0] + (int) a[1] });
    }

    private static byte[] readResource(String path) throws IOException {
        try (InputStream in = VirtualComputerExample.class.getResourceAsStream(path)) {
            if (in == null) {
                throw new IOException("リソースが見つかりません: " + path);
            }
            return in.readAllBytes();
        }
    }

    private VirtualComputerExample() {
    }
}
