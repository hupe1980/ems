//! Browser exports for the household solar and industrial peak simulations.
//! 0 starts now, 1 waits two minutes. Unavailable results return −1.
include!(concat!(env!("OUT_DIR"), "/peak_receipts.rs"));

fn decision(choice: i32) -> Option<&'static (i32, f64, f64)> {
    CHOICES.get(usize::try_from(choice).ok()?)
}

// All exports receive and return scalars. The host never owns a Rust allocation.
#[unsafe(no_mangle)]
pub extern "C" fn target_kw() -> i32 {
    TARGET
}

#[unsafe(no_mangle)]
pub extern "C" fn battery_kw() -> i32 {
    BATTERY
}

#[unsafe(no_mangle)]
pub extern "C" fn minutes_left(choice: i32) -> i32 {
    decision(choice).map_or(-1, |(minutes, _, _)| *minutes)
}

#[unsafe(no_mangle)]
pub extern "C" fn delivered_kwh(choice: i32) -> f64 {
    decision(choice).map_or(-1.0, |(_, kwh, _)| *kwh)
}

#[unsafe(no_mangle)]
pub extern "C" fn projected_mean_kw(choice: i32) -> f64 {
    decision(choice).map_or(-1.0, |(_, _, kw)| *kw)
}

#[unsafe(no_mangle)]
pub extern "C" fn excess_kw(choice: i32) -> f64 {
    decision(choice).map_or(-1.0, |(_, _, kw)| (kw - f64::from(target_kw())).max(0.0))
}

#[unsafe(no_mangle)]
pub extern "C" fn closed_kwh() -> f64 {
    CLOSE.0
}

#[unsafe(no_mangle)]
pub extern "C" fn closed_mean_kw() -> f64 {
    CLOSE.1
}

/// Steady-state household charging setpoint, in watts. The site offers 10 W steps.
#[unsafe(no_mangle)]
pub extern "C" fn solar_charge_w(surplus_w: i32, phases: i32) -> i32 {
    static SETPOINTS: &[[u16; 2]] = &include!(concat!(env!("OUT_DIR"), "/solar_setpoints.rs"));
    let column = match phases {
        1 => 0,
        3 => 1,
        _ => return -1,
    };
    if surplus_w < 0 || surplus_w % 10 != 0 {
        return -1;
    }
    SETPOINTS
        .get((surplus_w / 10) as usize)
        .map_or(-1, |row| i32::from(row[column]))
}
