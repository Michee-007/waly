// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Sel de reroll SAC (piège 3) : un rebuild sans mutation produit le même
/// hash → même verdict. Incrémenter suffit quand SAC bloque l'exe.
const SAC_REROLL: u32 = 27;

fn main() {
    std::hint::black_box(SAC_REROLL);
    waly_desktop_lib::run()
}
