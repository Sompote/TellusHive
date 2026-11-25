//! Runs each design module on a case and prints every step (symbol = value unit), so the
//! numbers can be checked against independent hand calculations.
use serde_json::json;

fn run(label: &str, module: &str, args: serde_json::Value) {
    println!("\n### {label}  [{module}]");
    match th_eng::design::run_module(module, &args) {
        Err(e) => println!("  ERROR: {e}"),
        Ok(d) => {
            for s in &d.sheet.steps {
                let chk = s.check.as_ref().map(|c| format!("  <{}>", serde_json::to_string(c).unwrap_or_default())).unwrap_or_default();
                println!("  {:<14} = {:>12.4} {:<8} {}{}", s.symbol, s.value, s.unit, s.reference, chk.chars().take(140).collect::<String>());
            }
            for n in &d.sheet.notes {
                println!("  note: {n}");
            }
            println!("  PASS={}  summary: {}", d.pass, d.summary);
        }
    }
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    let all = which.is_empty();
    if all || which == "rc" {
        run("R1 flexure 3DB20 300x500 Mu=150", "rc_beam_aci318", json!({"h": 500, "n_bars": 3, "Mu": 150}));
        run("R2 shear Vu=120, design s", "rc_beam_aci318", json!({"h": 500, "n_bars": 3, "Mu": 100, "Vu": 120}));
        run("R3 4DB28 transition zone", "rc_beam_aci318", json!({"h": 500, "n_bars": 4, "bar_dia": 28, "Mu": 200}));
        run("R4 Vu huge (section too small)", "rc_beam_aci318", json!({"h": 500, "n_bars": 3, "Mu": 100, "Vu": 900}));
        run("R5 bad input h=0", "rc_beam_aci318", json!({"h": 0, "n_bars": 3, "Mu": 100}));
    }
    if all || which == "terz" {
        run("T1 strip B=2 D=1.5 phi30 c0 g18", "bearing_terzaghi", json!({"B": 2, "D": 1.5, "phi": 30, "gamma": 18, "shape": "strip"}));
        run("T2 square B=2 D=1 phi25 c20 g17", "bearing_terzaghi", json!({"B": 2, "D": 1, "phi": 25, "c": 20, "gamma": 17, "shape": "square"}));
        run("T3 square phi0 cu50 B2 D1 g18", "bearing_terzaghi", json!({"B": 2, "D": 1, "phi": 0, "c": 50, "gamma": 18, "shape": "square"}));
        run("T4 T1 with water at base zw=1.5 gsat20", "bearing_terzaghi", json!({"B": 2, "D": 1.5, "phi": 30, "gamma": 18, "gamma_sat": 20, "z_w": 1.5, "shape": "strip"}));
        run("T5 T1 with water at surface zw=0", "bearing_terzaghi", json!({"B": 2, "D": 1.5, "phi": 30, "gamma": 18, "gamma_sat": 20, "z_w": 0, "shape": "strip"}));
        run("T6 phi 55 (out of range)", "bearing_terzaghi", json!({"B": 2, "phi": 55, "gamma": 18}));
        run("T7 rect L<B", "bearing_terzaghi", json!({"B": 3, "L": 2, "phi": 30, "gamma": 18, "shape": "rect"}));
    }
    if all || which == "ec7" {
        run("E1 square 2x2 D1.5 phi32 g18 Gk900 Qk300", "bearing_ec7", json!({"B": 2, "D": 1.5, "phi_k": 32, "gamma": 18, "Gk": 900, "Qk": 300}));
        run("E2 E1 with e_B=0.2", "bearing_ec7", json!({"B": 2, "D": 1.5, "phi_k": 32, "gamma": 18, "Gk": 900, "Qk": 300, "e_B": 0.2}));
        run("E3 undrained cu50 square 2x2 D1", "bearing_ec7", json!({"B": 2, "D": 1, "gamma": 18, "Gk": 500, "Qk": 100, "cu_k": 50}));
        run("E5 e_B = 0.7 > B/3", "bearing_ec7", json!({"B": 2, "D": 1, "phi_k": 30, "gamma": 18, "Gk": 300, "e_B": 0.7}));
        run("E6 e_B = -0.2 (other side)", "bearing_ec7", json!({"B": 2, "D": 1, "phi_k": 30, "gamma": 18, "Gk": 300, "e_B": -0.2}));
        run("E4 e_B too large (B/2)", "bearing_ec7", json!({"B": 2, "D": 1, "phi_k": 30, "gamma": 18, "Gk": 500, "e_B": 1.0}));
    }
    if all || which == "wall" {
        run("W1 H=4 toe0.8 heel2.2 D1.0 phi1=30 phi2=20 c2=40", "retaining_wall_cantilever", json!({"H": 4, "toe": 0.8, "heel": 2.2, "D": 1.0, "c2": 40}));
        run("W3 SIZE the base: H=4 q=10 D=1 c2=40", "retaining_wall_cantilever", json!({"H": 4, "D": 1.0, "c2": 40, "q": 10}));
        run("W4 SIZE, impossible soil (phi2=5, c2=0)", "retaining_wall_cantilever", json!({"H": 6, "D": 0.5, "phi2": 5, "c2": 0, "q": 20}));
        run("W5 k1=1 k2=0 (agent's choice)", "retaining_wall_cantilever", json!({"H": 4, "toe": 0.8, "heel": 2.2, "D": 1.0, "c2": 40, "q": 10, "k1": 1.0, "k2": 0.0}));
        run("W2 W1 + q=10", "retaining_wall_cantilever", json!({"H": 4, "toe": 0.8, "heel": 2.2, "D": 1.0, "c2": 40, "q": 10}));
    }
}
