#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs::File,
    io::{BufReader, Write},
};

use eframe::egui::{self, Id, TextEdit, vec2};

fn init_db() -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open("infra_items.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS infra_item (
            id INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            vendor TEXT NOT NULL,
            price REAL NOT NULL
        )",
        [],
    )?;
    Ok(conn)
}

fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "Catálogo Elétrico de Preços",
        options,
        Box::new(|cc| {
            let app = MyApp::new(cc);
            Ok(Box::new(app))
        }),
    )
}

struct MyApp {
    conn: rusqlite::Connection,
    selected_item_id: Option<i32>,
    items: Vec<InfraItem>,
    new_description: String,
    new_vendor: String,
    new_price: String,
    status_message: Option<String>,
    copied_feedback_timer: Option<std::time::Instant>,
    search_query: String,
}

impl MyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::light(); // or .dark()
        visuals.selection.bg_fill = egui::Color32::from_rgb(0, 100, 200);
        visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        cc.egui_ctx.set_visuals(visuals);

        let conn = init_db().unwrap();
        let mut app = MyApp {
            conn,
            selected_item_id: None,
            items: vec![],
            new_description: String::new(),
            new_vendor: String::new(),
            new_price: String::new(),
            status_message: None,
            copied_feedback_timer: None,
            search_query: String::new(),
        };
        app.load_items();
        app
    }
    fn load_items(&mut self) {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, vendor, price FROM infra_item ORDER BY id DESC")
            .unwrap();

        let item_iter = stmt
            .query_map([], |row| {
                Ok(InfraItem {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    vendor: row.get(2)?,
                    price: row.get(3)?,
                })
            })
            .unwrap();
        self.items = item_iter.filter_map(Result::ok).collect();
    }

    fn insert_item(&mut self, description: &str, vendor: &str, price: f32) {
        match self.conn.execute(
            "INSERT INTO infra_item (description, vendor, price) VALUES (?1, ?2, ?3)",
            (description, vendor, price),
        ) {
            Ok(rows) => {
                self.status_message = Some(format!("{} item inserido", rows));
                self.load_items();
            }
            Err(err) => {
                self.status_message = Some(format!("Erro ao inserir: {}", err));
            }
        }
    }

    fn update_item(&mut self) {
        if let Some(id) = self.selected_item_id {
            let parsed_price = self.new_price.replace(",", ".").parse::<f32>();

            match parsed_price {
                Ok(price) => {
                    let result = self.conn.execute(
                        "UPDATE infra_item SET description = ?1, vendor = ?2, price = ?3 WHERE id = ?4",
                        (
                            &self.new_description,
                            &self.new_vendor,
                            price,
                            id,
                        ),
                    );

                    match result {
                        Ok(updated_rows) => {
                            if updated_rows == 1 {
                                self.status_message = Some("Item atualizado".to_owned());
                                self.load_items();
                                // Optional: clear input fields
                                self.new_description.clear();
                                self.new_vendor.clear();
                                self.new_price.clear();
                                self.selected_item_id = None;
                            } else {
                                self.status_message =
                                    Some("Nenhum item foi atualizado.".to_owned());
                            }
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Erro ao atualizar:\n{}", e));
                        }
                    }
                }
                Err(_) => {
                    self.status_message = Some(format!("Preço inválido ou vazio"));
                }
            }
        } else {
            self.status_message = Some("Nenhum item selecionado.".to_owned());
        }
    }

    fn delete_selected_item(&mut self) {
        if let Some(id) = self.selected_item_id {
            let result = self
                .conn
                .execute("DELETE FROM infra_item WHERE id = ?1", [id]);

            match result {
                Ok(affected) if affected == 1 => {
                    self.status_message = Some("Item excluído com sucesso.".to_string());
                    self.selected_item_id = None;
                    self.load_items(); // Refresh the list
                    self.new_description.clear();
                    self.new_vendor.clear();
                    self.new_price.clear();
                }
                Ok(_) => {
                    self.status_message = Some("Nenhum item foi excluído.".to_string());
                }
                Err(e) => {
                    self.status_message = Some(format!("Erro ao excluir:\n{}", e));
                }
            }
        } else {
            self.status_message = Some("Nenhum item selecionado para excluir.".to_string());
        }
    }

    pub fn import_csv_to_db(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b';')
            .has_headers(true)
            .from_reader(BufReader::new(file));

        let tx = self.conn.transaction()?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO infra_item (description, vendor, price) VALUES (?1, ?2, ?3)",
            )?;

            for (index, result) in rdr.records().enumerate() {
                let record = result?;
                let description = record.get(0).unwrap_or("").trim();
                let vendor = record.get(1).unwrap_or("").trim();
                let price_str = record.get(2).unwrap_or("0").trim().replace(",", ".");
                let price: f32 = match price_str.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("❌ Preço inválido na linha {}: '{}'", index + 2, price_str);
                        continue;
                    }
                };

                stmt.execute(rusqlite::params![description, vendor, price])?;
            }
        } // <- Aqui stmt é dropado, e o compilador libera a referência a tx

        tx.commit()?; // <- Agora pode consumir `tx`

        self.load_items(); // <- Agora pode usar `self` de novo
        self.status_message = Some("CSV importado com sucesso.".to_string());

        Ok(())
    }

    pub fn export_to_csv(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = File::create(path)?;
        file.write_all(b"\xEF\xBB\xBF")?;
        let mut wtr = csv::WriterBuilder::new().delimiter(b';').from_writer(file);

        // Cabeçalho
        wtr.write_record(&["descrição", "fornecedor", "preço"])?;

        // Escreve os itens
        for item in &self.items {
            let preco = format!("{:.2}", item.price).replace(".", ","); // BR style
            wtr.write_record(&[&item.description, &item.vendor, &preco])?;
        }

        wtr.flush()?;
        self.status_message = Some("Exportado com sucesso.".into());
        Ok(())
    }
}

struct InfraItem {
    id: i32,
    description: String,
    vendor: String,
    price: f32,
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(msg) = &self.status_message {
            let status_label = msg.clone();
            egui::Window::new("Mensagem")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 30.0])
                .show(ctx, |ui| {
                    ui.label(status_label);
                    if ui.button("Fechar").clicked() {
                        self.status_message = None;
                    }
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Cadastro de Materiais Elétricos");

            egui::Grid::new("frm_cadastro")
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    let desired_text_with = 400.0;
                    ui.label("Descrição:");
                    ui.add(
                        TextEdit::singleline(&mut self.new_description)
                            .min_size(vec2(desired_text_with, 0.0)),
                    );
                    ui.end_row();

                    ui.label("Fornecedor:");
                    ui.add(
                        TextEdit::singleline(&mut self.new_vendor)
                            .min_size(vec2(desired_text_with, 0.0)),
                    );
                    ui.end_row();

                    ui.label("Preço (R$):");
                    ui.add(
                        TextEdit::singleline(&mut self.new_price)
                            .min_size(vec2(desired_text_with, 0.0)),
                    );
                    ui.end_row();
                });

            ui.horizontal(|ui| {
                if ui.button("Adicionar").clicked() {
                    if let Ok(price) = self.new_price.replace(",", ".").parse::<f32>() {
                        if !self.new_description.is_empty() && !self.new_vendor.is_empty() {
                            self.insert_item(
                                &self.new_description.clone(),
                                &self.new_vendor.clone(),
                                price,
                            );
                            self.new_description.clear();
                            self.new_vendor.clear();
                            self.new_price.clear();
                        }
                    }
                }

                if ui.button("Atualizar").clicked() {
                    self.update_item();
                }

                if ui.button("Excluir").clicked() {
                    self.delete_selected_item();
                }

                if ui.button("Importar CSV").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CSV files", &["csv"])
                        .pick_file()
                    {
                        if let Err(e) = self.import_csv_to_db(&path.to_string_lossy()) {
                            self.status_message = Some(format!("Erro ao importar: {}", e));
                        }
                    }
                }

                if ui.button("Exportar CSV").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CSV", &["csv"])
                        .set_file_name("catalogo.csv")
                        .save_file()
                    {
                        if let Err(e) = self.export_to_csv(&path.to_string_lossy()) {
                            self.status_message = Some(format!("Falha ao exportar: {}", e));
                        }
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Buscar:");
                ui.add(
                    TextEdit::singleline(&mut self.search_query)
                        .hint_text("Item ou fornecedor")
                        .min_size(vec2(300.0, 0.0)),
                );
                if ui.button("Limpar Pesquisa").clicked() {
                    self.search_query.clear();
                }
            });

            ui.label("Itens Cadastrados:");

            let search = self.search_query.to_lowercase();

            egui::ScrollArea::vertical()
                .max_width(ui.available_width())
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for item in self.items.iter().filter(|item| {
                        item.description.to_lowercase().contains(&search)
                            || item.vendor.to_lowercase().contains(&search)
                    }) {
                        let is_selected = Some(item.id) == self.selected_item_id;
                        let price_str = format!("{:.2}", item.price).replace(".", ",");
                        let label =
                            format!("[{}] {} R$ {}", item.vendor, item.description, price_str);
                        let selectable_label_response = ui.selectable_label(is_selected, &label);
                        if selectable_label_response.clicked_by(egui::PointerButton::Primary) {
                            if self.selected_item_id == Some(item.id) {
                                self.selected_item_id = None; // unselect if clicked again
                                self.new_description.clear();
                                self.new_vendor.clear();
                                self.new_price.clear();
                            } else {
                                self.selected_item_id = Some(item.id); // select item
                                self.new_description = item.description.clone();
                                self.new_vendor = item.vendor.clone();
                                self.new_price = price_str.clone();
                            }
                        }
                        if selectable_label_response.clicked_by(egui::PointerButton::Secondary) {
                            let label_to_copy = format!(
                                "{}\t\t\t\t{}\t{}",
                                item.description, item.vendor, price_str
                            );
                            ctx.copy_text(label_to_copy);
                            self.copied_feedback_timer = Some(std::time::Instant::now());
                        }
                    }
                });
            if let Some(t) = self.copied_feedback_timer {
                if t.elapsed().as_secs_f32() < 0.2 {
                    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        egui::Area::new(Id::new("copied_tooltip"))
                            .fixed_pos(pos + vec2(10.0, 10.0))
                            .interactable(false)
                            .show(ctx, |ui| {
                                ui.label("Copiado");
                            });
                    }
                } else {
                    self.copied_feedback_timer = None;
                }
            }
        });
    }
}
