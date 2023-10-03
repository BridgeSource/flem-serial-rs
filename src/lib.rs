use flem::Status;
use serialport::SerialPort;
use std::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

type FlemSerialPort = Box<dyn SerialPort>;
type FlemSerialTx = Option<Arc<Mutex<FlemSerialPort>>>;

pub enum HostSerialPortErrors {
    NoDeviceFoundByThatName,
    MultipleDevicesFoundByThatName,
    ErrorConnectingToDevice,
}

pub struct FlemSerial<const PACKET_SIZE: usize, const BUFFER_SIZE: usize> {
    tx_port: FlemSerialTx,
    continue_listening: Arc<Mutex<bool>>,
    rx_listener_handle: Option<JoinHandle<()>>,
    tx_listener_handle: Option<JoinHandle<()>>,
}

pub struct FlemRx<const PACKET_SIZE: usize> {
    rx_packet_queue: Receiver<flem::Packet<PACKET_SIZE>>,
}

impl<const PACKET_SIZE: usize> FlemRx<PACKET_SIZE> {
    pub fn recv(&self) -> Option<flem::Packet<PACKET_SIZE>> {
        match self.rx_packet_queue.recv() {
            Ok(packet) => Some(packet),
            Err(_error) => None,
        }
    }
}

#[derive(Clone)]
pub struct FlemTx<const PACKET_SIZE: usize> {
    tx_packet_queue: Sender<flem::Packet<PACKET_SIZE>>,
}

impl<const PACKET_SIZE: usize> FlemTx<PACKET_SIZE> {
    pub fn send(
        &self,
        packet: &flem::Packet<PACKET_SIZE>,
    ) -> Result<(), mpsc::SendError<flem::Packet<PACKET_SIZE>>> {
        self.tx_packet_queue.send(*packet)
    }

    pub fn clone(&self) -> Self {
        Self {
            tx_packet_queue: self.tx_packet_queue.clone(),
        }
    }
}

impl<const PACKET_SIZE: usize, const BUFFER_SIZE: usize> FlemSerial<PACKET_SIZE, BUFFER_SIZE> {
    pub fn new() -> Self {
        Self {
            tx_port: None,
            continue_listening: Arc::new(Mutex::new(false)),
            rx_listener_handle: None,
            tx_listener_handle: None,
        }
    }

    /// Lists the ports detected by the SerialPort library. Returns None if
    /// no serial ports are detected.
    pub fn list_serial_ports(&self) -> Option<Vec<String>> {
        let mut vec_ports = Vec::new();

        let ports = serialport::available_ports();

        match ports {
            Ok(valid_ports) => {
                for port in valid_ports {
                    println!("Found serial port: {}", port.port_name);
                    vec_ports.push(port.port_name);
                }
                return Some(vec_ports);
            }
            Err(_error) => {
                return None;
            }
        }
    }

    /// Attempts to connect to a serial port with a set baud.
    ///
    /// Configures the serial port with the following settings:
    /// * FlowControl: None
    /// * Parity: None
    /// * DataBits: 8
    /// * StopBits: 1
    pub fn connect(&mut self, port_name: &String, baud: u32) -> Result<(), HostSerialPortErrors> {
        let ports = serialport::available_ports().unwrap();

        let filtered_ports: Vec<_> = ports
            .iter()
            .filter(|port| port.port_name == *port_name)
            .collect();

        match filtered_ports.len() {
            0 => {
                println!("No serial devices enumerated");
                Err(HostSerialPortErrors::NoDeviceFoundByThatName)
            }
            1 => {
                println!("Found {}, attempting to connect...", port_name);
                if let Ok(port) = serialport::new(port_name, baud)
                    .flow_control(serialport::FlowControl::None)
                    .parity(serialport::Parity::None)
                    .data_bits(serialport::DataBits::Eight)
                    .stop_bits(serialport::StopBits::One)
                    .open()
                {
                    self.tx_port = Some(Arc::new(Mutex::new(
                        port.try_clone()
                            .expect("Couldn't clone serial port for tx_port"),
                    )));

                    println!("Connection successful to {}", port_name);

                    return Ok(());
                } else {
                    println!("Connection failed to {}", port_name);
                    return Err(HostSerialPortErrors::ErrorConnectingToDevice);
                }
            }
            _ => {
                println!(
                    "Connection failed to {}, multiple devices by that name found",
                    port_name
                );
                Err(HostSerialPortErrors::MultipleDevicesFoundByThatName)
            }
        }
    }

    pub fn disconnect(&mut self) -> Option<()> {
        self.unlisten();

        Some(())
    }

    /// Spawns a new thread and listens for data on. Creates 2 threads, 1 to monitor incoming Rx bytes and 1 to monitor
    /// packets that have been queued up to transmit.
    ///
    /// ## Arguments
    /// * `rx_sleep_time_ms` - The amount of time to sleep between reads if there is no data present.
    /// * `tx_sleep_time_ms` - The amount of time to sleep between checking the packet queue if no tx packets are present.
    ///
    /// ## Returns a tuple of (FlemRx, FlemTx):
    /// * `FlemRx` - A struct that contains a `Receiver` queue of successfully checksumed packets and a handle to the Rx monitoring thread.
    /// * `FlemTx` - A struct that contains a `Sender` that can be used to send packets and a handle to the Tx monitoring thread.
    pub fn listen(
        &mut self,
        rx_sleep_time_ms: u64,
        tx_sleep_time_ms: u64,
    ) -> (FlemRx<PACKET_SIZE>, FlemTx<PACKET_SIZE>) {
        // Reset the continue_listening flag
        *self.continue_listening.lock().unwrap() = true;

        // Clone the continue_listening flag
        let continue_listening_clone_rx = self.continue_listening.clone();
        let continue_listening_clone_tx = self.continue_listening.clone();

        // Create producer / consumer queues
        let (successful_packet_queue, rx) = mpsc::channel::<flem::Packet<PACKET_SIZE>>();
        let (tx_packet_queue, tx) = mpsc::channel::<flem::Packet<PACKET_SIZE>>();

        let mut local_rx_port = self
            .tx_port
            .as_mut()
            .unwrap()
            .lock()
            .unwrap()
            .try_clone()
            .expect("Couldn't clone serial port for rx_port");

        let mut local_tx_port = self
            .tx_port
            .as_mut()
            .unwrap()
            .lock()
            .unwrap()
            .try_clone()
            .expect("Couldn't clone serial port for tx_port");

        self.tx_listener_handle = Some(thread::spawn(move || {
            println!("Starting Tx thread");
            while *continue_listening_clone_tx.lock().unwrap() {
                // Using a timeout so we can stop the thread when needed
                match tx.recv_timeout(Duration::from_millis(tx_sleep_time_ms)) {
                    Ok(packet) => {
                        if let Ok(_) = local_tx_port.write_all(&packet.bytes()) {
                            local_tx_port.flush().unwrap();
                        }
                    }
                    Err(_error) => {
                        // Timeouts are ok, just keep checking the Tx queue
                    }
                }
            }

            println!("Tx thread stopped");
            *continue_listening_clone_tx.lock().unwrap() = false;
        }));

        self.rx_listener_handle = Some(thread::spawn(move || {
            println!("Starting Rx thread");

            let mut rx_buffer = [0 as u8; BUFFER_SIZE];
            let mut rx_packet = flem::Packet::<PACKET_SIZE>::new();

            while *continue_listening_clone_rx.lock().unwrap() {
                match local_rx_port.read(&mut rx_buffer) {
                    Ok(bytes_to_read) => {
                        // Check if there are any bytes, if there are no bytes,
                        // put the thread to sleep
                        if bytes_to_read == 0 {
                            thread::sleep(Duration::from_millis(10));
                        } else {
                            for i in 0..bytes_to_read {
                                match rx_packet.construct(rx_buffer[i]) {
                                    Ok(_) => {
                                        successful_packet_queue.send(rx_packet.clone()).unwrap();
                                        rx_packet.reset_lazy();
                                    }
                                    Err(error) => {
                                        match error {
                                            Status::PacketBuilding => {
                                                // Normal, building packet DO NOT RESET!
                                            }
                                            Status::HeaderBytesNotFound => {
                                                // Not unusual, keep scanning until packet lock occurs
                                            }
                                            Status::ChecksumError => {
                                                println!("FLEM checksum error detected");
                                                rx_packet.reset_lazy();
                                            }
                                            _ => {
                                                println!("FLEM error detected: {:?}", error);
                                                rx_packet.reset_lazy();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(_error) => {
                        // Library indicates to retry on errors, so that is
                        // what we will do.
                        thread::sleep(Duration::from_millis(rx_sleep_time_ms));
                    }
                }
            }

            println!("Rx thread stopped");
            *continue_listening_clone_rx.lock().unwrap() = false;
        }));

        (
            FlemRx {
                rx_packet_queue: rx,
            },
            FlemTx {
                tx_packet_queue: tx_packet_queue,
            },
        )
    }

    /// Attempts to close down the Rx and Tx threads.
    pub fn unlisten(&mut self) {
        *self.continue_listening.lock().unwrap() = false;

        if self.tx_listener_handle.is_some() {
            self.tx_listener_handle
                .take()
                .unwrap()
                .join()
                .expect("Couldn't join on the Tx thread");
        }

        if self.rx_listener_handle.is_some() {
            self.rx_listener_handle
                .take()
                .unwrap()
                .join()
                .expect("Couldn't join on Rx thread");
        }
    }
}
