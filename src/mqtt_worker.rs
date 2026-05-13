//! Tâche Tokio de fond pour la réception MQTT et le stockage en base.
//!
//! Démarrage automatique depuis main.rs si MQTT_BROKER_URL est défini dans .env.
//! Aucun plugin nécessaire — les données sont lues via plugin_sql + config_actions.json.
//!
//! ## Variables .env
//! ```
//! MQTT_BROKER_URL=localhost        ← host du broker (sans tcp://)
//! MQTT_BROKER_PORT=1883            ← port (défaut 1883)
//! MQTT_CLIENT_ID=small-folks       ← identifiant du client
//! MQTT_TOPICS=sensors/#            ← topics séparés par virgule
//! MQTT_QOS=1                       ← 0, 1 ou 2
//! ```
//!
//! ## Formats de messages supportés
//! - `sensors/temperature`  → payload float : "25.3"
//! - `sensors/humidity`     → payload float : "60.5"
//! - `sensors/+/data`       → payload JSON  : {"sensor_id":"DHT22-001","temperature":25.3,"humidity":60.5}
//!
//! Les topics temperature/humidity sont mis en buffer par sensor_id.
//! L'insertion en base a lieu quand les deux valeurs sont disponibles.

use sqlx::MySqlPool;
use std::collections::HashMap;
use std::time::Duration;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};

// ── Point d'entrée appelé depuis main.rs ──────────────────────────────────────

/// Démarre la tâche MQTT en fond.
/// Boucle de reconnexion automatique toutes les 5s en cas d'erreur.
pub async fn start(pool: MySqlPool) {
    let broker_host = std::env::var("MQTT_BROKER_URL")
        .unwrap_or_else(|_| "localhost".to_string());
    let broker_port = std::env::var("MQTT_BROKER_PORT")
        .unwrap_or_else(|_| "1883".to_string())
        .parse::<u16>()
        .unwrap_or(1883);
    let client_id = std::env::var("MQTT_CLIENT_ID")
        .unwrap_or_else(|_| "small-folks-mqtt".to_string());
    let topics_str = std::env::var("MQTT_TOPICS")
        .unwrap_or_else(|_| "sensors/#".to_string());
    let qos_val = std::env::var("MQTT_QOS")
        .unwrap_or_else(|_| "1".to_string())
        .parse::<u8>()
        .unwrap_or(1);

    let qos = match qos_val {
        0 => QoS::AtMostOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtLeastOnce,
    };

    let topics: Vec<String> = topics_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    println!("[mqtt_worker] Broker    : {}:{}", broker_host, broker_port);
    println!("[mqtt_worker] Client ID : {}", client_id);
    println!("[mqtt_worker] Topics    : {}", topics.join(", "));
    println!("[mqtt_worker] QoS       : {}", qos_val);

    // Boucle de reconnexion automatique
    loop {
        println!("[mqtt_worker] Connexion au broker...");
        match run_loop(&pool, &broker_host, broker_port, &client_id, &topics, qos).await {
            Ok(_) => {
                println!("[mqtt_worker] Boucle terminée, reconnexion dans 5s...");
            }
            Err(e) => {
                eprintln!("[mqtt_worker] Erreur : {}", e);
                eprintln!("[mqtt_worker] Reconnexion dans 5s...");
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

// ── Boucle principale MQTT ────────────────────────────────────────────────────

async fn run_loop(
    pool:     &MySqlPool,
    host:     &str,
    port:     u16,
    cid:      &str,
    topics:   &[String],
    qos:      QoS,
) -> Result<(), Box<dyn std::error::Error>> {

    let mut opts = MqttOptions::new(cid, host, port);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(true);

    // Channel de 100 messages en attente
    let (client, mut eventloop) = AsyncClient::new(opts, 100);

    // Souscription aux topics
    for topic in topics {
        client.subscribe(topic.as_str(), qos).await?;
        println!("[mqtt_worker] ✓ Souscrit : {}", topic);
    }

    println!("[mqtt_worker] En attente de messages...");

    // Buffer température/humidité par sensor_id
    // (pour les topics individuels sensors/temperature et sensors/humidity)
    let mut buffer: HashMap<String, SensorBuffer> = HashMap::new();

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(msg))) => {
                let topic   = msg.topic.as_str();
                let payload = std::str::from_utf8(&msg.payload)
                    .unwrap_or("")
                    .trim()
                    .to_string();

                handle_message(pool, topic, &payload, &mut buffer).await;
            }
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                println!("[mqtt_worker] ✓ Connecté au broker");
            }
            Ok(Event::Incoming(Packet::Disconnect)) => {
                println!("[mqtt_worker] Broker déconnecté");
                return Ok(());
            }
            Err(e) => {
                return Err(Box::new(e));
            }
            _ => {}
        }
    }
}

// ── Traitement d'un message ───────────────────────────────────────────────────

async fn handle_message(
    pool:    &MySqlPool,
    topic:   &str,
    payload: &str,
    buffer:  &mut HashMap<String, SensorBuffer>,
) {
    // ── Format JSON combiné : sensors/<id>/data ───────────────────────────────
    // Payload : {"sensor_id":"DHT22-001","temperature":25.3,"humidity":60.5}
    if topic.ends_with("/data") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
            if let (Some(sid), Some(temp), Some(hum)) = (
                json["sensor_id"].as_str(),
                json["temperature"].as_f64(),
                json["humidity"].as_f64(),
            ) {
                insert_capteur(pool, sid, temp as f32, hum as f32).await;
            } else {
                eprintln!("[mqtt_worker] JSON incomplet sur {}: {}", topic, payload);
            }
        } else {
            eprintln!("[mqtt_worker] JSON invalide sur {}: {}", topic, payload);
        }
        return;
    }

    // ── Topics individuels : sensors/temperature et sensors/humidity ──────────
    // On buffer par sensor_id (défaut : "default" si pas de sous-topic)
    // Ex: sensors/DHT22-001/temperature → sensor_id = "DHT22-001"
    //     sensors/temperature           → sensor_id = "default"
    let parts: Vec<&str> = topic.split('/').collect();
    let sensor_id = if parts.len() >= 3 {
        parts[1].to_string()
    } else {
        "default".to_string()
    };

    let entry = buffer.entry(sensor_id.clone()).or_default();

    if topic.contains("temperature") {
        if let Ok(v) = payload.parse::<f32>() {
            entry.temperature = Some(v);
            println!("[mqtt_worker] Température {} : {:.1}°C", sensor_id, v);
            if v > 30.0 { println!("[mqtt_worker] ⚠️  ALERTE température élevée !"); }
        }
    } else if topic.contains("humidity") {
        if let Ok(v) = payload.parse::<f32>() {
            entry.humidity = Some(v);
            println!("[mqtt_worker] Humidité {} : {:.1}%", sensor_id, v);
            if v > 80.0 { println!("[mqtt_worker] ⚠️  ALERTE humidité élevée !"); }
        }
    }

    // Si les deux valeurs sont disponibles → INSERT et vider le buffer
    if let (Some(temp), Some(hum)) = (entry.temperature, entry.humidity) {
        insert_capteur(pool, &sensor_id, temp, hum).await;
        buffer.remove(&sensor_id);
    }
}

// ── INSERT en base ────────────────────────────────────────────────────────────

async fn insert_capteur(
    pool:        &MySqlPool,
    sensor_id:   &str,
    temperature: f32,
    humidity:    f32,
) {
    let now = chrono::Local::now().naive_local();
    match sqlx::query(
        "INSERT INTO capteurs (sensor_id, temperature, humidity, timestamp)
         VALUES (?, ?, ?, ?)"
    )
    .bind(sensor_id)
    .bind(temperature)
    .bind(humidity)
    .bind(now)
    .execute(pool)
    .await {
        Ok(r) => println!(
            "[mqtt_worker] ✓ INSERT capteurs id={} sensor={} t={:.1}°C h={:.1}%",
            r.last_insert_id(), sensor_id, temperature, humidity
        ),
        Err(e) => eprintln!("[mqtt_worker] ✗ Erreur INSERT : {}", e),
    }
}

// ── Structures internes ───────────────────────────────────────────────────────

#[derive(Default)]
struct SensorBuffer {
    temperature: Option<f32>,
    humidity:    Option<f32>,
}
