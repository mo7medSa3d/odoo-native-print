fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "print_pdf",
            "print_text",
            "print_image",
            "print_status",
            "job_detail",
            "app_overview",
            "list_printers",
            "printer_forms",
            "run_test_print",
            "retry_job_cmd",
            "cancel_job_cmd",
            "clear_finished_cmd",
            "export_diagnostics",
            "save_origin_cmd",
            "open_control_center",
            "open_odoo_origin",
        ]),
    );
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
