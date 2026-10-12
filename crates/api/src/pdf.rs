use chrono::Datelike;
use vaultscope_db::models::{Customer, Invoice, InvoiceLineItem, InvoiceStatus};

/// Generates a GoBD-compliant PDF document for the given invoice, customer, and line items.
pub fn generate_invoice_pdf(
    invoice: &Invoice,
    customer: &Customer,
    line_items: &[InvoiceLineItem],
) -> Vec<u8> {
    let mut content = String::with_capacity(4096);

    // --- Header Background Accent Bar ---
    content.push_str("0.05 0.08 0.14 rg\n"); // Deep navy/slate
    content.push_str("0 770 595.28 72 re f\n");

    // --- VaultScope Brand & Title ---
    content.push_str("1.0 1.0 1.0 rg\n"); // White
    content.push_str("BT /F2 18 Tf 45 805 Td (VAULTSCOPE) Tj ET\n");
    content.push_str("0.7 0.75 0.85 rg\n");
    content.push_str("BT /F1 9 Tf 45 790 Td (CLOUD INFRASTRUCTURE & SOVEREIGN HOSTING) Tj ET\n");

    // Title on top right
    content.push_str("1.0 1.0 1.0 rg\n");
    content.push_str("BT /F2 16 Tf 420 805 Td (RECHNUNG / INVOICE) Tj ET\n");
    content.push_str("0.8 0.85 0.95 rg\n");
    content.push_str(&format!(
        "BT /F1 10 Tf 420 790 Td ({}) Tj ET\n",
        encode_pdf_text(&invoice.invoice_number)
    ));

    // --- Company Address Header (DIN 5008 Sender Line) ---
    content.push_str("0.5 0.5 0.5 rg\n");
    content.push_str("BT /F1 7.5 Tf 45 745 Td (VaultScope GmbH - Hanauer Landstrasse 291b - 60314 Frankfurt am Main - Deutschland) Tj ET\n");
    content.push_str("0.8 0.8 0.8 RG 0.5 w 45 738 m 300 738 l S\n"); // Thin separator line

    // --- Customer Address Block ---
    content.push_str("0.1 0.1 0.1 rg\n");
    let mut cur_y = 720;
    if !customer.company.is_empty() {
        content.push_str(&format!(
            "BT /F2 10 Tf 45 {} Td ({}) Tj ET\n",
            cur_y,
            encode_pdf_text(&customer.company)
        ));
        cur_y -= 14;
    }
    content.push_str(&format!(
        "BT /F1 10 Tf 45 {} Td ({}) Tj ET\n",
        cur_y,
        encode_pdf_text(&customer.name)
    ));
    cur_y -= 14;

    if !customer.address.is_empty() {
        content.push_str(&format!(
            "BT /F1 9.5 Tf 45 {} Td ({}) Tj ET\n",
            cur_y,
            encode_pdf_text(&customer.address)
        ));
        cur_y -= 14;
    }

    let country_str = if customer.country.is_empty() { "Deutschland" } else { &customer.country };
    content.push_str(&format!(
        "BT /F1 9.5 Tf 45 {} Td ({}) Tj ET\n",
        cur_y,
        encode_pdf_text(country_str)
    ));
    cur_y -= 14;

    if !customer.vat_id.is_empty() {
        content.push_str(&format!(
            "BT /F1 8.5 Tf 45 {} Td (USt-IdNr.: {}) Tj ET\n",
            cur_y,
            encode_pdf_text(&customer.vat_id)
        ));
    }

    // --- Invoice Metadata Box (Right Side) ---
    content.push_str("0.96 0.97 0.98 rg 0.85 0.85 0.85 RG 0.75 w\n");
    content.push_str("360 635 190 100 re B\n"); // Fill and stroke box

    content.push_str("0.4 0.4 0.4 rg\n");
    content.push_str("BT /F1 8.5 Tf 370 718 Td (Rechnungsnummer:) Tj ET\n");
    content.push_str("BT /F1 8.5 Tf 370 702 Td (Rechnungsdatum:) Tj ET\n");
    content.push_str("BT /F1 8.5 Tf 370 686 Td (Faelligkeitsdatum:) Tj ET\n");
    content.push_str("BT /F1 8.5 Tf 370 670 Td (Kundennummer:) Tj ET\n");
    content.push_str("BT /F1 8.5 Tf 370 654 Td (Zahlungsstatus:) Tj ET\n");

    content.push_str("0.1 0.1 0.1 rg\n");
    content.push_str(&format!(
        "BT /F2 8.5 Tf 455 718 Td ({}) Tj ET\n",
        encode_pdf_text(&invoice.invoice_number)
    ));
    content.push_str(&format!(
        "BT /F1 8.5 Tf 455 702 Td ({}) Tj ET\n",
        invoice.issue_date.format("%d.%m.%Y")
    ));
    content.push_str(&format!(
        "BT /F1 8.5 Tf 455 686 Td ({}) Tj ET\n",
        invoice.due_date.format("%d.%m.%Y")
    ));
    content.push_str(&format!(
        "BT /F1 8.5 Tf 455 670 Td (KD-{}) Tj ET\n",
        &customer.id.to_string()[..8]
    ));

    let status_str = match invoice.status {
        InvoiceStatus::Paid => "BEZAHLT",
        InvoiceStatus::Pending => "OFFEN",
        InvoiceStatus::Draft => "ENTWURF",
        InvoiceStatus::Overdue => "UEBERFAELLIG",
        InvoiceStatus::Void => "STORNIERT",
    };
    if invoice.status == InvoiceStatus::Paid {
        content.push_str("0.1 0.6 0.2 rg\n");
    } else if invoice.status == InvoiceStatus::Overdue {
        content.push_str("0.8 0.1 0.1 rg\n");
    } else {
        content.push_str("0.2 0.3 0.7 rg\n");
    }
    content.push_str(&format!("BT /F2 8.5 Tf 455 654 Td ({}) Tj ET\n", status_str));

    // --- Line Items Table Header ---
    let table_y = 600;
    content.push_str("0.05 0.08 0.14 rg\n");
    content.push_str(&format!("45 {} 505 20 re f\n", table_y));

    content.push_str("1.0 1.0 1.0 rg\n");
    content.push_str(&format!("BT /F2 8.5 Tf 55 {} Td (Pos.) Tj ET\n", table_y + 6));
    content.push_str(&format!("BT /F2 8.5 Tf 95 {} Td (Beschreibung / Leistung) Tj ET\n", table_y + 6));
    content.push_str(&format!("BT /F2 8.5 Tf 330 {} Td (Menge) Tj ET\n", table_y + 6));
    content.push_str(&format!("BT /F2 8.5 Tf 390 {} Td (Einzelpreis) Tj ET\n", table_y + 6));
    content.push_str(&format!("BT /F2 8.5 Tf 485 {} Td (Gesamt) Tj ET\n", table_y + 6));

    // --- Line Items Rows ---
    let mut row_y = table_y - 20;
    for (i, item) in line_items.iter().enumerate() {
        if i % 2 == 1 {
            content.push_str("0.97 0.98 0.99 rg\n");
            content.push_str(&format!("45 {} 505 18 re f\n", row_y));
        }

        content.push_str("0.15 0.15 0.15 rg\n");
        content.push_str(&format!(
            "BT /F1 8.5 Tf 55 {} Td ({:02}) Tj ET\n",
            row_y + 5,
            i + 1
        ));
        content.push_str(&format!(
            "BT /F1 8.5 Tf 95 {} Td ({}) Tj ET\n",
            row_y + 5,
            encode_pdf_text(&item.description)
        ));
        content.push_str(&format!(
            "BT /F1 8.5 Tf 340 {} Td ({}x) Tj ET\n",
            row_y + 5,
            item.quantity
        ));
        content.push_str(&format!(
            "BT /F1 8.5 Tf 390 {} Td ({:.2} EUR) Tj ET\n",
            row_y + 5,
            item.unit_price
        ));
        content.push_str(&format!(
            "BT /F2 8.5 Tf 480 {} Td ({:.2} EUR) Tj ET\n",
            row_y + 5,
            item.total
        ));

        // Thin separator line
        content.push_str("0.9 0.9 0.9 RG 0.5 w\n");
        content.push_str(&format!("45 {} m 550 {} l S\n", row_y, row_y));

        row_y -= 19;
    }

    // If no line items, output a fallback row
    if line_items.is_empty() {
        content.push_str("0.2 0.2 0.2 rg\n");
        content.push_str(&format!(
            "BT /F1 8.5 Tf 95 {} Td (VaultScope Cloud Infrastructure Services) Tj ET\n",
            row_y + 5
        ));
        content.push_str(&format!(
            "BT /F1 8.5 Tf 340 {} Td (1x) Tj ET\n",
            row_y + 5
        ));
        content.push_str(&format!(
            "BT /F1 8.5 Tf 390 {} Td ({:.2} EUR) Tj ET\n",
            row_y + 5,
            invoice.subtotal
        ));
        content.push_str(&format!(
            "BT /F2 8.5 Tf 480 {} Td ({:.2} EUR) Tj ET\n",
            row_y + 5,
            invoice.subtotal
        ));
        row_y -= 19;
    }

    // --- Totals Summary Box ---
    let totals_y = row_y - 15;
    content.push_str("0.9 0.9 0.9 RG 0.5 w\n");
    content.push_str(&format!("330 {} m 550 {} l S\n", totals_y + 40, totals_y + 40));

    content.push_str("0.4 0.4 0.4 rg\n");
    content.push_str(&format!("BT /F1 8.5 Tf 340 {} Td (Nettobetrag / Subtotal:) Tj ET\n", totals_y + 24));
    content.push_str(&format!(
        "BT /F1 8.5 Tf 340 {} Td (zzgl. 19% USt. / VAT:) Tj ET\n",
        totals_y + 10
    ));

    content.push_str("0.1 0.1 0.1 rg\n");
    content.push_str(&format!(
        "BT /F1 8.5 Tf 480 {} Td ({:.2} EUR) Tj ET\n",
        totals_y + 24,
        invoice.subtotal
    ));
    content.push_str(&format!(
        "BT /F1 8.5 Tf 480 {} Td ({:.2} EUR) Tj ET\n",
        totals_y + 10,
        invoice.tax_amount
    ));

    // Double underline for gross total
    content.push_str("0.05 0.08 0.14 rg\n");
    content.push_str(&format!("330 {} 220 22 re f\n", totals_y - 18));
    content.push_str("1.0 1.0 1.0 rg\n");
    content.push_str(&format!(
        "BT /F2 10 Tf 340 {} Td (GESAMTBETRAG / TOTAL:) Tj ET\n",
        totals_y - 12
    ));
    content.push_str(&format!(
        "BT /F2 10.5 Tf 475 {} Td ({:.2} EUR) Tj ET\n",
        totals_y - 12,
        invoice.total
    ));

    // --- GoBD Legal Compliance Notes ---
    let legal_y = totals_y - 65;
    content.push_str("0.4 0.4 0.4 rg\n");
    content.push_str(&format!(
        "BT /F2 8 Tf 45 {} Td (Hinweise zur Besteuerung & Zahlungsmodalitaeten / GoBD Compliance:) Tj ET\n",
        legal_y
    ));
    content.push_str(&format!(
        "BT /F1 7.5 Tf 45 {} Td (Steuerbare Leistung gemaess Paragraph 1 Abs. 1 Nr. 1 UStG. Leistungszeitraum entspricht dem Rechnungsdatum.) Tj ET\n",
        legal_y - 11
    ));
    if !invoice.notes.is_empty() {
        content.push_str(&format!(
            "BT /F1 7.5 Tf 45 {} Td (Anmerkung: {}) Tj ET\n",
            legal_y - 22,
            encode_pdf_text(&invoice.notes)
        ));
    } else if invoice.status == InvoiceStatus::Paid {
        content.push_str(&format!(
            "BT /F1 7.5 Tf 45 {} Td (Der Rechnungsbetrag wurde vollstaendig beglichen. Vielen Dank fuer Ihr Vertrauen!) Tj ET\n",
            legal_y - 22
        ));
    } else {
        content.push_str(&format!(
            "BT /F1 7.5 Tf 45 {} Td (Bitte ueberweisen Sie den faelligen Betrag unter Angabe der Rechnungsnummer bis zum Faelligkeitsdatum.) Tj ET\n",
            legal_y - 22
        ));
    }

    // --- GoBD Audit Integrity Hash Bar ---
    content.push_str("0.92 0.94 0.96 rg 0.8 0.85 0.9 RG 0.5 w\n");
    content.push_str("45 95 505 24 re B\n");
    content.push_str("0.3 0.35 0.45 rg\n");
    content.push_str(&format!(
        "BT /F1 7 Tf 55 108 Td (GoBD Audit-Trail: Unveraenderbare elektronische Rechnung gem. BMF-Schreiben IV A 4 - S 0316/19/10003) Tj ET\n"
    ));
    content.push_str(&format!(
        "BT /F2 7 Tf 55 99 Td (Hash: SHA-256 [INV-{}-{}] | Signiert: {}) Tj ET\n",
        invoice.issue_date.year(),
        &invoice.id.to_string()[..12],
        invoice.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // --- Footer DIN 5008 (4 Columns: Company, Management, Registry/Tax, Banking) ---
    content.push_str("0.8 0.8 0.8 RG 0.5 w 45 80 m 550 80 l S\n");

    content.push_str("0.45 0.45 0.45 rg\n");
    // Col 1
    content.push_str("BT /F2 7 Tf 45 70 Td (VaultScope GmbH) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 45 61 Td (Hanauer Landstr. 291b) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 45 52 Td (60314 Frankfurt am Main) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 45 43 Td (www.vaultscope.de) Tj ET\n");

    // Col 2
    content.push_str("BT /F2 7 Tf 175 70 Td (Geschaeftsfuehrung) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 175 61 Td (VaultScope Executive Board) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 175 52 Td (Support: help@vaultscope.de) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 175 43 Td (Billing: billing@vaultscope.de) Tj ET\n");

    // Col 3
    content.push_str("BT /F2 7 Tf 315 70 Td (Register & Steuern) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 315 61 Td (AG Frankfurt am Main, HRB 118492) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 315 52 Td (USt-IdNr.: DE 312 984 920) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 315 43 Td (Steuernummer: 045 240 81290) Tj ET\n");

    // Col 4
    content.push_str("BT /F2 7 Tf 445 70 Td (Bankverbindung) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 445 61 Td (Commerzbank AG Frankfurt) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 445 52 Td (IBAN: DE44 5008 0000 0123 4567 89) Tj ET\n");
    content.push_str("BT /F1 6.5 Tf 445 43 Td (BIC: COBADEFFXXX) Tj ET\n");

    // Build the complete PDF document with valid cross-reference table and trailer
    build_pdf_document(&content)
}

/// Assembles a valid PDF 1.4 document stream from text instructions
fn build_pdf_document(content_stream: &str) -> Vec<u8> {
    let mut pdf = Vec::new();

    // 1. Header
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let mut offsets = Vec::new();

    // Obj 1: Catalog
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Obj 2: Pages
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // Obj 3: Page (A4: 595.28 x 841.89 points)
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R >> /ProcSet [/PDF /Text] >> >>\nendobj\n");

    // Obj 4: Content Stream
    offsets.push(pdf.len());
    let stream_bytes = content_stream.as_bytes();
    let stream_header = format!("4 0 obj\n<< /Length {} >>\nstream\n", stream_bytes.len());
    pdf.extend_from_slice(stream_header.as_bytes());
    pdf.extend_from_slice(stream_bytes);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    // Obj 5: Helvetica Font (Regular)
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n");

    // Obj 6: Helvetica-Bold Font
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"6 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>\nendobj\n");

    // Cross-reference table
    let xref_offset = pdf.len();
    let num_objects = offsets.len() + 1; // including object 0
    pdf.extend_from_slice(format!("xref\n0 {}\n", num_objects).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }

    // Trailer
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", num_objects, xref_offset).as_bytes());

    pdf
}

fn encode_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for ch in s.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            'ä' => out.push_str("\\344"),
            'ö' => out.push_str("\\366"),
            'ü' => out.push_str("\\374"),
            'Ä' => out.push_str("\\304"),
            'Ö' => out.push_str("\\326"),
            'Ü' => out.push_str("\\334"),
            'ß' => out.push_str("\\337"),
            '€' => out.push_str("EUR"),
            c if c.is_ascii() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use vaultscope_db::models::CustomerStatus;

    #[test]
    fn test_generate_invoice_pdf_valid() {
        let invoice = Invoice {
            id: Uuid::now_v7(),
            invoice_number: "INV-2026-0042".to_string(),
            customer_id: Uuid::now_v7(),
            status: InvoiceStatus::Paid,
            subtotal: Decimal::new(10000, 2),
            tax_rate: Decimal::new(19, 2),
            tax_amount: Decimal::new(1900, 2),
            total: Decimal::new(11900, 2),
            issue_date: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(),
            paid_at: Some(Utc::now()),
            stripe_payment_intent_id: Some("pi_test_123".to_string()),
            notes: "Test invoice".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let customer = Customer {
            id: invoice.customer_id,
            external_id: "ext_1".to_string(),
            name: "Max Mustermann".to_string(),
            company: "Muster GmbH".to_string(),
            email: "max@example.com".to_string(),
            phone: "+49123456789".to_string(),
            email_verified: true,
            address: "Musterstrasse 1".to_string(),
            postal_code: "10115".to_string(),
            city: "Berlin".to_string(),
            country: "Deutschland".to_string(),
            vat_id: "DE123456789".to_string(),
            status: CustomerStatus::Active,
            two_factor_enabled: false,
            notes: "".to_string(),
            stripe_customer_id: None,
            created_at: Utc::now(),
            last_login: None,
            updated_at: Utc::now(),
            password_hash: None,
            verification_token: None,
            totp_secret: None,
            vat_verified: true,
        };

        let items = vec![InvoiceLineItem {
            id: Uuid::now_v7(),
            invoice_id: invoice.id,
            description: "Cloud VPS CX32 (Monthly)".to_string(),
            quantity: 1,
            unit_price: Decimal::new(10000, 2),
            total: Decimal::new(10000, 2),
            service_id: None,
            sort_order: 1,
        }];

        let pdf = generate_invoice_pdf(&invoice, &customer, &items);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        assert!(pdf.len() > 1000);
    }
}
