fn main() {
    embuild::espidf::sysenv::output();

    println!("cargo::rerun-if-changed=.env");
    if let Ok(iter) = dotenvy::dotenv_iter() {
        for item in iter {
            if let Ok((key, val)) = item {
                println!("cargo::rustc-env={}={}", key, val);
            }
        }
    }
}
