use flem::{
    Status,
    traits::Channel, 
    Packet,
};
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

pub struct FlemSerial<const PACKET_SIZE: usize> {
    tx_port: FlemSerialTx,
    continue_listening: Arc<Mutex<bool>>,
    rx_listener_handle: Option<JoinHandle<()>>,
    tx_listener_handle: Option<JoinHandle<()>>,
    connection_settings: ConnectionSettings,
}
#[derive(Debug)]
pub enum FlemSerialErrors {
    NoDeviceFoundByThatName,
    MultipleDevicesFoundByThatName,
    ErrorConnectingToDevice,
}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionSettings {
    baud: u32,
    parity: serialport::Parity,
    flow_control: serialport::FlowControl,
    data_bits: serialport::DataBits,
    stop_bits: serialport::StopBits,
}

impl ConnectionSettings {
    pub fn default() -> Self {
        Self {
            baud: 115200,
            parity: serialport::Parity::None,
            flow_control: serialport::FlowControl::None,
            data_bits: serialport::DataBits::Eight,
            stop_bits: serialport::StopBits::One,
        }
    }

    /// Set baud rate
    pub fn baud(&mut self, baud: u32) -> &mut Self {
        self.baud = baud;
        self
    }

    /// Set parity
    pub fn parity(&mut self, parity: serialport::Parity) -> &mut Self {
        self.parity = parity;
        self
    }

    /// Set flow control
    pub fn flow_control(&mut self, flow_control: serialport::FlowControl) -> &mut Self {
        self.flow_control = flow_control;
        self
    }

    /// Set data bits
    pub fn data_bits(&mut self, data_bits: serialport::DataBits) -> &mut Self {
        self.data_bits = data_bits;
        self
    }

    /// Set stop bits
    pub fn stop_bits(&mut self, stop_bits: serialport::StopBits) -> &mut Self {
        self.stop_bits = stop_bits;
        self
    }
}

impl<const PACKET_SIZE: usize> FlemSerial<PACKET_SIZE> {
    pub fn new() -> Self {
        Self {
            tx_port: None,
            continue_listening: Arc::new(Mutex::new(false)),
            rx_listener_handle: None,
            tx_listener_handle: None,
            connection_settings: ConnectionSettings::default(),
        }
    }

    pub fn update_connection_settings(&mut self, connection_settings: ConnectionSettings) {
        self.connection_settings = connection_settings;
    }
}

impl<const PACKET_SIZE: usize> Channel<PACKET_SIZE> for FlemSerial<PACKET_SIZE> {
    type Error = FlemSerialErrors;

    /// Lists the ports detected by the SerialPort library. Returns None if
    /// no serial ports are detected.
    fn list_devices(&self) -> Vec<String> {
        let mut vec_ports = Vec::new();

        let ports = serialport::available_ports();

        match ports {
            Ok(valid_ports) => {
                for port in valid_ports {
                    println!("Found serial port: {}", port.port_name);
                    vec_ports.push(port.port_name);
                }
            }
            Err(_error) => {

            }
        }

        vec_ports
    }

    /// Attempts to connect to a serial port with a set baud.
    ///
    /// Configures the serial port with the following settings:
    /// * FlowControl: None
    /// * Parity: None
    /// * DataBits: 8
    /// * StopBits: 1
    fn connect(&mut self, port_name: &String) -> Result<(), Self::Error> {
        let ports = serialport::available_ports().unwrap();

        let filtered_ports: Vec<_> = ports
            .iter()
            .filter(|port| port.port_name == *port_name)
            .collect();

        match filtered_ports.len() {
            0 => {
                println!("No serial devices enumerated");
                Err(FlemSerialErrors::NoDeviceFoundByThatName)
            }
            1 => {
                println!("Found {}, attempting to connect...", port_name);
                if let Ok(port) = serialport::new(port_name, self.connection_settings.baud)
                    .flow_control(self.connection_settings.flow_control)
                    .parity(self.connection_settings.parity)
                    .data_bits(self.connection_settings.data_bits)
                    .stop_bits(self.connection_settings.stop_bits)
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
                    return Err(FlemSerialErrors::ErrorConnectingToDevice);
                }
            }
            _ => {
                println!(
                    "Connection failed to {}, multiple devices by that name found",
                    port_name
                );
                Err(FlemSerialErrors::MultipleDevicesFoundByThatName)
            }
        }
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        self.unlisten().unwrap();

        self.tx_port = None;

        Ok(())
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
    fn listen(
        &mut self,
        rx_sleep_time_ms: u64,
        tx_sleep_time_ms: u64,
    ) -> (Sender<Packet<PACKET_SIZE>>, Receiver<Packet<PACKET_SIZE>>) {
        // Reset the continue_listening flag
        *self.continue_listening.lock().unwrap() = true;

        // Clone the continue_listening flag
        let continue_listening_clone_rx = self.continue_listening.clone();
        let continue_listening_clone_tx = self.continue_listening.clone();

        // Create producer / consumer queues
        let (tx_packet_from_program, packet_to_transmit) = mpsc::channel::<flem::Packet<PACKET_SIZE>>();
        let (validated_packet, rx_packet_to_program) = mpsc::channel::<flem::Packet<PACKET_SIZE>>();

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
                match packet_to_transmit.recv_timeout(Duration::from_millis(tx_sleep_time_ms)) {
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

            let mut rx_buffer = [0 as u8; 2048];
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
                                        validated_packet.send(rx_packet.clone()).unwrap();
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
            tx_packet_from_program,
            rx_packet_to_program
        )
    }

    /// Attempts to close down the Rx and Tx threads.
    fn unlisten(&mut self) -> Result<(), Self::Error>{
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

        Ok(())
    }
}
