use flem::traits::Channel;
use std::{io, thread, time::Duration};

// Usually, embedded targets have a smaller packet size, for example 128 bytes of data (and 8 bytes for the header).
// The packet size is the WORST CASE size, and FLEM packets can easily handle smaller packets.
const PACKET_SIZE: usize = 512;

fn main() {
    let mut flem_serial = flem_serial_rs::FlemSerial::<PACKET_SIZE>::new();

    let mut input_buffer = String::new();

    let mut selection_invalid = true;

    let mut selected_port: Option<String> = None;

    while selection_invalid {
        let ports = flem_serial.list_devices();
        match ports.len() {
            0 => {
                println!("No serial ports detected, press any key to quit...");
                match io::stdin().read_line(&mut input_buffer) {
                    Ok(_) => {
                        return;
                    }
                    Err(_) => {
                        return;
                    }
                }
            }
            _ => {
                let mut line = 0;
                for port in ports.iter() {
                    println!("{}. {}", line, port);
                    line += 1;
                }
                println!("{}. Quit", line);
                println!("Select port: ");

                input_buffer.clear();

                // Select the serial port to use
                match io::stdin().read_line(&mut input_buffer) {
                    Ok(_characters) => {
                        match input_buffer.trim().parse::<usize>() {
                            Ok(selection) => {
                                if selection < line {
                                    // Selection is valid
                                    selection_invalid = false;
                                    selected_port = Some(ports[selection].clone());
                                    println!("Connecting to port {}", ports[selection]);
                                } else if selection == line {
                                    // quit the program
                                    return;
                                } else {
                                    // Repeat the selection
                                }
                            }
                            Err(parse_error) => {
                                // Bad parse, repeat the selection
                                println!("Error parsing selection: {}", parse_error.to_string());
                            }
                        }
                    }
                    Err(read_line_error) => {
                        println!(
                            "Error reading line, exiting program: {}",
                            read_line_error.to_string()
                        );
                        return;
                    }
                }
            }
        }
    }

    let port_name = &selected_port.unwrap();
    match flem_serial.connect(port_name) {
        Ok(_) => {}
        Err(_) => {
            println!(
                "Error connecting to serial port {} with error, exiting program",
                port_name
            );
            return;
        }
    }

    // Start the FLEM serial listener threads. This will spawn two threads to handle Rx and Tx of FLEM packets.
    // The Rx struct is backed by a queue that will only be populated with packets that pass the CRC check.
    // The Tx struct can be cloned and used by any thread to send packets.
    let (flem_tx, flem_rx) = flem_serial.listen(10, 50);

    // Any thread that would like to send a packet can clone the Tx Queue
    let _flem_tx_clone = flem_tx.clone();
    let flem_tx_clone_1 = flem_tx.clone();
    //...
    let _flem_tx_clone_10 = flem_tx.clone();

    // Note - This example doesn't properly shut down the threads, just kill the program
    println!("Starting Tx broadcast thread - Press Enter to send an ID packet");
    let _uart_tx_thread = thread::spawn(move || loop {
        println!("Press Enter to send an ID packet");
        match io::stdin().read_line(&mut input_buffer) {
            Ok(_) => {
                println!("Sending ID packet request");
                let mut packet = flem::Packet::<PACKET_SIZE>::new();
                packet.set_request(flem::request::ID);
                packet.pack();
                flem_tx_clone_1.send(packet).unwrap();
            }
            Err(_) => {}
        }
    });

    // Note - This example doesn't properly shut down the threads, just kill the program
    println!("Starting RX listening thread");
    let _uart_rx_thread_processor = thread::spawn(move || {
        loop {
            match flem_rx.recv() {
                Ok(packet) => {
                    // Any packet received is guaranteed to be validated, so you just need to handle the
                    // requests and events.
                    let packet_data = &packet.get_data();
                    match packet.get_request() {
                        flem::request::ID => {
                            let id: flem::DataId = flem::DataId::from(packet_data).unwrap();
                            println!(
                                "Flem Device: {:?}, version {}.{}.{}, packet size: {}",
                                id.get_name(),
                                id.get_major(),
                                id.get_minor(),
                                id.get_patch(),
                                id.get_max_packet_size()
                            );
                        }
                        _ => {
                            // TODO - Handle other requests
                            println!("Unknown command");
                        }
                    }
                }
                Err(_) => {
                    // Wait for data
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
    });

    // This won't be hit since the two threads above never quite, but if they did...
    _uart_rx_thread_processor.join().unwrap();
    _uart_tx_thread.join().unwrap();

    // If we were to properly close down the `_uart_rx_thread_processor` and `_uart_tx_thread`, we would call
    // the `.unlisten()` function to close the serial port backing threads.
    let _ = flem_serial.unlisten();
}
