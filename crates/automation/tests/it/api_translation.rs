//! Synthetic fixtures only; every outbound request targets an ephemeral loopback mock.
use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
fn fixture() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 4 0 R >> >> >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        {
            let body = "BT /F1 16 Tf 20 150 Td (Lamination 1.6mm) Tj ET";
            format!("<< /Length {} >>\nstream\n{}\nendstream", body.len(), body)
        },
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for o in offsets {
        bytes.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}
fn result(out: Vec<Content>) -> Value {
    out.into_iter().find_map(|c| if let Content::Json(v) = c { Some(v) } else { None }).unwrap()
}
fn mock(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let start = Instant::now();
            let mut stream = loop {
                if let Ok((s, _)) = listener.accept() {
                    break s;
                }
                assert!(start.elapsed() < Duration::from_secs(15), "mock did not receive request");
                std::thread::sleep(Duration::from_millis(10));
            };
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 8192];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                assert!(bytes.len() < 12 * 1024 * 1024);
                if let Some((h, b)) = String::from_utf8_lossy(&bytes).split_once("\r\n\r\n") {
                    let size = h
                        .lines()
                        .find_map(|l| {
                            l.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if b.len() >= size {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            requests.push(serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap());
            let response = json!({"choices":[{"finish_reason":"stop","message":{"content":reply.to_string()}}]}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        }
        requests
    });
    (url, handle)
}
static ID: AtomicU64 = AtomicU64::new(0);
#[test]
fn api_translation_text_image_permissions_confinement_and_unchanged_document() {
    let path = std::env::temp_dir().join(format!("pdf-api-test-{}-{}", std::process::id(), ID.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).unwrap();
    let bytes = fixture();
    std::fs::write(path.join("synthetic.pdf"), &bytes).unwrap();
    let mut a = Automation::new().with_root(&path).unwrap();
    let opened = result(a.call("doc_open", &json!({"path":"synthetic.pdf"})).unwrap());
    let doc = opened["doc"].clone();
    let before = result(a.call("doc_info", &json!({"doc":doc})).unwrap());
    let text = result(a.call("api_selection_text", &json!({"doc":doc,"page":1,"start":0,"end":16})).unwrap());
    assert_eq!(text["text"], "Lamination 1.6mm");
    assert_eq!(text["sent"], false);
    assert!(a.call("api_selection_text", &json!({"doc":doc,"page":1,"start":0,"end":999})).is_err());
    let crop_args = json!({"doc":doc,"page":1,"mode":"image","rect":[10,120,190,170],"rotation":90});
    let crop = a.call("api_selection_image", &crop_args).unwrap();
    assert!(crop.iter().any(|c| matches!(c, Content::Png { width: 100, height: 360, .. })));
    let (url, server) = mock(vec![
        json!({"translation":"压合 1.6mm","notes":""}),
        json!({"source_text":"Lamination 1.6mm","notes":""}),
        json!({"translation":"压合 1.6mm","notes":""}),
    ]);
    let profile = json!({"baseUrl":url,"model":"text-model","visionModel":"vision-model"});
    let mut args = json!({"doc":doc,"page":1,"start":0,"end":16,"direction":"en_zh","profile":profile});
    assert!(a.call("api_translate", &args).unwrap_err().to_string().contains("allow_api_send"));
    args["allow_api_send"] = json!(true);
    args["glossary_path"] = json!("../private-glossary.json");
    assert!(a.call("api_translate", &args).unwrap_err().to_string().contains("outside"));
    let glossary = json!([{"en":"Lamination","zh":"压合"}]).to_string();
    std::fs::write(path.join("terms.json"), glossary).unwrap();
    args["glossary_path"] = json!("terms.json");
    let translated = result(a.call("api_translate", &args).unwrap());
    assert_eq!(translated["matched_terms"], 1);
    args["mode"] = json!("image");
    args["rect"] = crop_args["rect"].clone();
    args["rotation"] = json!(90);
    let image = result(a.call("api_translate", &args).unwrap());
    assert_eq!(image["source_text"], "Lamination 1.6mm");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1]["model"], "vision-model");
    assert!(requests[1]["messages"][0]["content"][1]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    let data: Value = serde_json::from_str(requests[2]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(data["pcb_glossary"][0]["en"], "Lamination");
    let after = result(a.call("doc_info", &json!({"doc":doc})).unwrap());
    assert_eq!(before, after);
    assert_eq!(std::fs::read(path.join("synthetic.pdf")).unwrap(), bytes);
    // Permission denial precedes extraction and any outbound operation.
    a.call("doc_protect", &json!({"doc":doc,"open_password":"open","permissions_password":"test-owner","copy":false})).unwrap();
    a.call("doc_save", &json!({"doc":doc,"path":"locked.pdf"})).unwrap();
    let mut locked = Automation::new().with_root(&path).unwrap();
    let locked_doc = result(locked.call("doc_open", &json!({"path":"locked.pdf","password":"open"})).unwrap())["doc"].clone();
    for (tool, mut denied) in
        [("api_selection_text", json!({"page":1,"start":0,"end":16})), ("api_selection_image", crop_args), ("api_translate", args)]
    {
        denied["doc"] = locked_doc.clone();
        assert!(locked.call(tool, &denied).unwrap_err().to_string().contains("don't allow copying"), "{tool} must reject before sending");
    }
    std::fs::remove_dir_all(&path).unwrap();
}
