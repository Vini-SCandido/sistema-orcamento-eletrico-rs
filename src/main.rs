#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs::File,
    io::{BufReader, Write},
};

use chrono::Utc;
use eframe::egui::{self, Button, TextEdit, vec2};
use num_format::{Locale, ToFormattedString};

fn init_db() -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open("infra_items.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS infra_item (
            id INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            brand TEXT NOT NULL,
            vendor TEXT NOT NULL,
            price REAL NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(description, brand, vendor)
        )",
        [],
    )?;
    Ok(conn)
}

pub fn format_money(valor: f32) -> String {
    let inteiro = valor.trunc() as u64;
    let centavos = format!("{:.2}", valor.fract())[2..].to_string(); // pega só os dígitos após o ponto
    format!("{},{}", inteiro.to_formatted_string(&Locale::de), centavos)
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
    visible_items: Vec<InfraItem>,
    new_description: String,
    new_brand: String,
    new_vendor: String,
    new_price: String,
    status_message: Option<String>,
    status_message_timer: Option<std::time::Instant>,
    // copied_feedback_timer: Option<std::time::Instant>,
    search_query: String,
    last_search_query: String,
    show_outdated: bool,
    confirm_delete: bool,
}

impl MyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::light(); // or .dark()
        visuals.selection.bg_fill = egui::Color32::from_rgb(255, 212, 128);
        visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        cc.egui_ctx.set_visuals(visuals);

        let conn = init_db().unwrap();
        let mut app = MyApp {
            conn,
            selected_item_id: None,
            items: vec![],
            visible_items: vec![],
            new_description: String::new(),
            new_brand: String::new(),
            new_vendor: String::new(),
            new_price: String::new(),
            status_message: None,
            status_message_timer: None,
            // copied_feedback_timer: None,
            search_query: String::new(),
            last_search_query: String::new(),
            show_outdated: false,
            confirm_delete: false,
        };
        app.load_items();
        app
    }

    fn load_items(&mut self) {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, brand, vendor, price, updated_at FROM infra_item ORDER BY id DESC")
            .unwrap();

        let item_iter = stmt
            .query_map([], |row| {
                Ok(InfraItem {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    brand: row.get(2)?,
                    vendor: row.get(3)?,
                    price: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .unwrap();
        self.items = item_iter.filter_map(Result::ok).collect();
        self.visible_items = self.items.clone();
    }

    pub fn load_outdated_items(&mut self) {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, description, brand, vendor, price, updated_at
                 FROM infra_item
                 WHERE updated_at < DATE('now', '-1 month')",
            )
            .unwrap();

        let item_iter = stmt
            .query_map([], |row| {
                Ok(InfraItem {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    brand: row.get(2)?,
                    vendor: row.get(3)?,
                    price: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .unwrap();

        self.items = item_iter.filter_map(Result::ok).collect();
        self.visible_items = self.items.clone();
    }

    fn insert_item(&mut self, description: &str, brand: &str, vendor: &str, price: f32) {
        let now = Utc::now().format("%Y-%m-%d").to_string();
        match self.conn.execute(
            "INSERT INTO infra_item (description, brand, vendor, price, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(description, brand, vendor) DO UPDATE SET
                price = excluded.price,
                updated_at = excluded.updated_at",
            (description, brand, vendor, price, now),
        ) {
            Ok(_) => {
                self.status_message = Some("Item inserido".to_string());
                self.status_message_timer = None;
                self.load_items();
            }
            Err(err) => {
                self.status_message = Some(format!("Erro ao inserir: {}", err));
                self.status_message_timer = None;
            }
        }
        self.selected_item_id = None;
    }

    fn update_item(&mut self) {
        if let Some(id) = self.selected_item_id {
            let now = Utc::now().format("%Y-%m-%d").to_string();
            let parsed_price = self.new_price.replace(",", ".").parse::<f32>();

            match parsed_price {
                Ok(price) => {
                    if let Some(original_item) = self.items.iter().find(|item| item.id == id) {
                        // Verifica se houve alguma mudança
                        let changed = self.new_description != original_item.description
                            || self.new_brand != original_item.brand
                            || self.new_vendor != original_item.vendor
                            || (price - original_item.price).abs() > f32::EPSILON;

                        if !changed {
                            self.status_message = Some("Nenhuma alteração detectada.".to_owned());
                            self.status_message_timer = None;
                            return;
                        }

                        // Executa o update
                        let result = self.conn.execute(
                        "UPDATE infra_item SET description = ?1, brand = ?2, vendor = ?3, price = ?4, updated_at = ?5 WHERE id = ?6",
                        (
                            &self.new_description,
                            &self.new_brand,
                            &self.new_vendor,
                            price,
                            now,
                            id,
                        ),
                    );

                        match result {
                            Ok(updated_rows) => {
                                if updated_rows == 1 {
                                    self.status_message = Some("Item atualizado.".to_owned());
                                    self.status_message_timer = None;
                                    self.load_items();
                                    self.new_description.clear();
                                    self.new_brand.clear();
                                    self.new_vendor.clear();
                                    self.new_price.clear();
                                    self.selected_item_id = None;
                                } else {
                                    self.status_message =
                                        Some("Nenhum item foi atualizado.".to_owned());
                                    self.status_message_timer = None;
                                }
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Erro ao atualizar:\n{}", e));
                                self.status_message_timer = None;
                            }
                        }
                    }
                }
                Err(_) => {
                    self.status_message = Some(format!("Preço inválido ou vazio"));
                    self.status_message_timer = None;
                }
            }
        } else {
            self.status_message = Some("Nenhum item selecionado.".to_owned());
            self.status_message_timer = None;
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
                    self.status_message_timer = None;
                    self.selected_item_id = None;
                    self.load_items(); // Refresh the list
                    self.new_description.clear();
                    self.new_brand.clear();
                    self.new_vendor.clear();
                    self.new_price.clear();
                }
                Ok(_) => {
                    self.status_message = Some("Nenhum item foi excluído.".to_string());
                    self.status_message_timer = None;
                }
                Err(e) => {
                    self.status_message = Some(format!("Erro ao excluir: {}", e));
                    self.status_message_timer = None;
                }
            }
        } else {
            self.status_message = Some("Nenhum item selecionado para excluir.".to_string());
            self.status_message_timer = None;
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
                "INSERT INTO infra_item (description, brand, vendor, price, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(description, brand, vendor) DO UPDATE SET
                    price = excluded.price,
                    updated_at = excluded.updated_at",
            )?;

            for (index, result) in rdr.records().enumerate() {
                let record = result?;
                let description = record.get(0).unwrap_or("").trim();
                let brand = record.get(1).unwrap_or("").trim();
                let vendor = record.get(2).unwrap_or("").trim();
                let price_str = record.get(3).unwrap_or("0").trim().replace(",", ".");
                let updated_at = record.get(4).unwrap_or("").trim();
                let price: f32 = match price_str.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        self.status_message = Some(format!(
                            "Preço inválido na linha {}: '{}'",
                            index + 2,
                            price_str
                        ));
                        self.status_message_timer = None;
                        continue;
                    }
                };

                stmt.execute(rusqlite::params![
                    description,
                    brand,
                    vendor,
                    price,
                    updated_at
                ])?;
            }
        } // <- Aqui stmt é dropado, e o compilador libera a referência a tx

        tx.commit()?; // <- Agora pode consumir `tx`

        self.load_items(); // <- Agora pode usar `self` de novo
        self.status_message = Some("CSV importado com sucesso.".to_string());
        self.status_message_timer = None;

        Ok(())
    }

    pub fn export_to_csv(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = File::create(path)?;
        file.write_all(b"\xEF\xBB\xBF")?;
        let mut wtr = csv::WriterBuilder::new().delimiter(b';').from_writer(file);

        // Cabeçalho
        wtr.write_record(&[
            "descrição",
            "marca",
            "fornecedor",
            "preço",
            "última atualização",
        ])?;

        // Escreve os itens
        for item in &self.items {
            let preco = format!("{:.2}", item.price).replace(".", ","); // BR style
            wtr.write_record(&[
                &item.description,
                &item.brand,
                &item.vendor,
                &preco,
                &item.updated_at,
            ])?;
        }

        wtr.flush()?;
        self.status_message = Some("Exportado com sucesso.".into());
        self.status_message_timer = None;
        Ok(())
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            if input.key_pressed(egui::Key::Escape) && self.selected_item_id.is_some() {
                self.selected_item_id = None;
                self.new_description.clear();
                self.new_brand.clear();
                self.new_vendor.clear();
                self.new_price.clear();
            }

            if input.key_pressed(egui::Key::Delete) {
                if self.selected_item_id.is_some() {
                    self.confirm_delete = true;
                }
            }
        });
    }
}

#[derive(Clone)]
struct InfraItem {
    id: i32,
    description: String,
    brand: String,
    vendor: String,
    price: f32,
    updated_at: String,
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(msg) = &self.status_message {
            if self.status_message_timer.is_none() {
                self.status_message_timer = Some(std::time::Instant::now());
            }
            let status_label = msg.clone();
            egui::Window::new("Notificação")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 30.0])
                .show(ctx, |ui| {
                    ui.label(status_label);
                });
            if let Some(t) = self.status_message_timer {
                if t.elapsed().as_secs_f32() > 3.0 {
                    self.status_message = None;
                    self.status_message_timer = None;
                }
            }
        }

        if self.confirm_delete {
            egui::Window::new("Confirmar exclusão")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Tem certeza que deseja excluir este item?");

                    ui.horizontal(|ui| {
                        if ui.button("Cancelar").clicked() {
                            self.confirm_delete = false;
                        }

                        if ui.button("Sim, excluir").clicked() {
                            self.delete_selected_item();
                            self.selected_item_id = None;
                            self.status_message = Some("Item excluído.".into());
                            self.status_message_timer = None;
                            self.confirm_delete = false;
                        }
                    });
                });
        }

        self.handle_keyboard_shortcuts(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!self.confirm_delete, |ui| {
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

                        ui.label("Marca:");
                        ui.add(
                            TextEdit::singleline(&mut self.new_brand)
                                .hint_text("Sem Marca")
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
                                    &self.new_brand.clone(),
                                    &self.new_vendor.clone(),
                                    price,
                                );
                                self.new_description.clear();
                                self.new_brand.clear();
                                self.new_vendor.clear();
                                self.new_price.clear();
                            } else {
                                self.status_message =
                                    Some("Campo de descrição ou fabricante está vazio".into());
                                self.status_message_timer = None;
                            }
                        }
                    }

                    if self.selected_item_id.is_some() {
                        if ui.button("Atualizar").clicked() {
                            if !self.new_description.is_empty() && !self.new_vendor.is_empty() {
                                self.update_item();
                            } else {
                                self.status_message =
                                    Some("Campo de descrição ou fabricante está vazio".into());
                                self.status_message_timer = None;
                            }
                        }
                    }

                    if self.selected_item_id.is_some() {
                        if ui.button("Excluir").clicked() {
                            // self.delete_selected_item();
                            self.confirm_delete = true;
                        }
                    }

                    if ui.button("Importar CSV").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("CSV files", &["csv"])
                            .pick_file()
                        {
                            if let Err(e) = self.import_csv_to_db(&path.to_string_lossy()) {
                                self.status_message = Some(format!("Erro ao importar: {}", e));
                                self.status_message_timer = None;
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
                                self.status_message_timer = None;
                            }
                        }
                    }

                    if ui
                        .checkbox(&mut self.show_outdated, "Exibir desatualizados")
                        .clicked()
                    {
                        if self.show_outdated {
                            self.load_outdated_items();
                        } else {
                            self.load_items();
                        }
                        self.selected_item_id = None;
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Buscar:");
                    ui.add(
                        TextEdit::singleline(&mut self.search_query)
                            .hint_text("Item, fornecedor ou marca")
                            .min_size(vec2(300.0, 0.0)),
                    );
                    if ui.button("Limpar Pesquisa").clicked() {
                        self.search_query.clear();
                    }
                });

                ui.label("Itens Cadastrados:");

                if self.search_query != self.last_search_query {
                    self.last_search_query = self.search_query.clone();

                    let search = self.search_query.to_lowercase();

                    self.visible_items = if search.is_empty() {
                        self.items.clone()
                    } else {
                        self.items
                            .iter()
                            .filter(|item| {
                                item.description.to_lowercase().contains(&search)
                                    || item.vendor.to_lowercase().contains(&search)
                                    || item.brand.to_lowercase().contains(&search)
                            })
                            .cloned()
                            .collect()
                    };
                }

                let row_height = 24.0;
                let total_rows = self.visible_items.len();

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show_rows(ui, row_height, total_rows, |ui, row_range| {
                        ui.set_width(ui.available_width());
                        for row in row_range {
                            if let Some(item) = self.visible_items.get(row) {
                                let is_selected = Some(item.id) == self.selected_item_id;
                                let price_str = format_money(item.price);
                                let brand_str = if !item.brand.is_empty() {
                                    format!(" [{}]", item.brand)
                                } else {
                                    "".to_string()
                                };
                                let label = format!(
                                    "[{}]{} {} R$ {} {}",
                                    item.vendor,
                                    brand_str,
                                    item.description,
                                    price_str,
                                    item.updated_at
                                );

                                let selectable_label_response = ui.add(
                                    Button::new(&label)
                                        .selected(is_selected)
                                        .min_size(vec2(row_height, 0.0)),
                                );
                                // ui.selectable_label(is_selected, &label);
                                if selectable_label_response
                                    .clicked_by(egui::PointerButton::Primary)
                                {
                                    if self.selected_item_id == Some(item.id) {
                                        self.selected_item_id = None; // unselect if clicked again
                                        self.new_description.clear();
                                        self.new_brand.clear();
                                        self.new_vendor.clear();
                                        self.new_price.clear();
                                    } else {
                                        self.selected_item_id = Some(item.id); // select item
                                        self.new_description = item.description.clone();
                                        self.new_brand = item.brand.clone();
                                        self.new_vendor = item.vendor.clone();
                                        self.new_price = price_str.clone().replace(".", "");
                                    }
                                }

                                if selectable_label_response
                                    .clicked_by(egui::PointerButton::Secondary)
                                {
                                    let label_to_copy = format!(
                                        "{} {}\t\t\t\t{}\t{}",
                                        item.description, item.brand, item.vendor, price_str
                                    );
                                    ctx.copy_text(label_to_copy);
                                    self.status_message =
                                        Some("Copiado para a área de transferência".into());
                                    self.status_message_timer = None;
                                    // self.copied_feedback_timer = Some(std::time::Instant::now());
                                }
                            }
                        }
                        // for item in self.items.iter().filter(|item| {
                        //     item.description.to_lowercase().contains(&search)
                        //         || item.vendor.to_lowercase().contains(&search)
                        //         || item.brand.to_lowercase().contains(&search)
                        // }) {
                        //     let is_selected = Some(item.id) == self.selected_item_id;
                        //     let price_str = format_money(item.price);
                        //     let brand_str = if !item.brand.is_empty() {
                        //         format!(" [{}]", item.brand)
                        //     } else {
                        //         "".to_string()
                        //     };
                        //     let label = format!(
                        //         "[{}]{} {} R$ {} {}",
                        //         item.vendor,
                        //         brand_str,
                        //         item.description,
                        //         price_str,
                        //         item.updated_at
                        //     );

                        //     let selectable_label_response =
                        //         ui.selectable_label(is_selected, &label);
                        //     if selectable_label_response.clicked_by(egui::PointerButton::Primary) {
                        //         if self.selected_item_id == Some(item.id) {
                        //             self.selected_item_id = None; // unselect if clicked again
                        //             self.new_description.clear();
                        //             self.new_brand.clear();
                        //             self.new_vendor.clear();
                        //             self.new_price.clear();
                        //         } else {
                        //             self.selected_item_id = Some(item.id); // select item
                        //             self.new_description = item.description.clone();
                        //             self.new_brand = item.brand.clone();
                        //             self.new_vendor = item.vendor.clone();
                        //             self.new_price = price_str.clone().replace(".", "");
                        //         }
                        //     }

                        //     if selectable_label_response.clicked_by(egui::PointerButton::Secondary)
                        //     {
                        //         let label_to_copy = format!(
                        //             "{} {}\t\t\t\t{}\t{}",
                        //             item.description, item.brand, item.vendor, price_str
                        //         );
                        //         ctx.copy_text(label_to_copy);
                        //         self.status_message =
                        //             Some("Copiado para a área de transferência".into());
                        //         self.status_message_timer = None;
                        //         // self.copied_feedback_timer = Some(std::time::Instant::now());
                        //     }
                        // }
                    });
                // if let Some(t) = self.copied_feedback_timer {
                //     if t.elapsed().as_secs_f32() < 0.2 {
                //         if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                //             egui::Area::new(Id::new("copied_tooltip"))
                //                 .fixed_pos(pos + vec2(10.0, 10.0))
                //                 .interactable(false)
                //                 .show(ctx, |ui| {
                //                     ui.label("Copiado");
                //                 });
                //         }
                //     } else {
                //         self.copied_feedback_timer = None;
                //     }
                // }
            });
        });
    }
}
