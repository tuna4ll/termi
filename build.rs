fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/termi.ico");
        res.set("ProductName", "Termi");
        res.set("FileDescription", "Termi Code Editor");
        res.set("LegalCopyright", "Copyright © 2026");
        res.compile().unwrap();
    }
}
