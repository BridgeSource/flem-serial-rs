use flem::Status;
use serialport::SerialPort;
use std::{
    sync::{
        mpsc::{self, Receiver},
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

pub struct FlemSerial<const T: usize> {
    tx_port: FlemSerialTx,
    continue_listening: Arc<Mutex<bool>>,
}

pub struct FlemRx<const T: usize> {
    rx_listener_handle: JoinHandle<()>,
    rx_packet_queue: Receiver<flem::Packet<T>>,
}

impl<const T: usize> FlemRx<T> {
    pub fn queue(&self) -> &Receiver<flem::Packet<T>> {
        &self.rx_packet_queue
    }

    pub fn join_handle(&self) -> &JoinHandle<()> {
        &self.rx_listener_handle
    }
}

impl<const T: usize> FlemSerial<T> {
    pub fn new() -> Self {
        Self {
            tx_port: None,
            continue_listening: Arc::new(Mutex::new(false)),
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
    pub fn connect(&mut self, port_name: &String, baud: u32) -> Result<(), HostSerialPortErrors> {
        let ports = serialport::available_ports().unwrap();

        let filtered_ports: Vec<_> = ports
            .iter()
            .filter(|port| port.port_name == *port_name)
            .collect();

        match filtered_ports.len() {
            0 => Err(HostSerialPortErrors::NoDeviceFoundByThatName),
            1 => {
                if let Ok(port) = serialport::new(port_name, baud)
                    .flow_control(serialport::FlowControl::None)
                    .parity(serialport::Parity::None)
                    .data_bits(serialport::DataBits::Eight)
                    .stop_bits(serialport::StopBits::One)
                    .timeout(Duration::from_millis(10))
                    .open()
                {
                    self.tx_port = Some(Arc::new(Mutex::new(
                        port.try_clone()
                            .expect("Couldn't clone serial port for tx_port"),
                    )));

                    return Ok(());
                } else {
                    return Err(HostSerialPortErrors::ErrorConnectingToDevice);
                }
            }
            _ => Err(HostSerialPortErrors::MultipleDevicesFoundByThatName),
        }
    }

    pub fn disconnect(&mut self) -> Option<()> {
        self.unlisten();

        Some(())
    }

    /// Spawns a new thread and listens for data on. Returns a handle to the
    /// thread that can be used to join later.
    ///
    /// Use [received_packets] to get a mpsc::Receiver of type flem::Packet::<T>
    pub fn listen(&mut self) -> FlemRx<T> {
        // Reset the continue_listening flag
        *self.continue_listening.lock().unwrap() = true;

        // Clone the continue_listening flag
        let continue_listening_clone = self.continue_listening.clone();

        // Create producer / consumer queues
        let (successful_packet_queue, rx) = mpsc::channel::<flem::Packet<T>>();

        let mut local_rx_port = self
            .tx_port
            .as_mut()
            .unwrap()
            .lock()
            .unwrap()
            .try_clone()
            .expect("Couldn't clone serial port for rx_port");

        let rx_thread_handle = thread::spawn(move || {
            let mut rx_buffer = [0 as u8; T];
            let mut rx_packet = flem::Packet::<T>::new();

            let error = 0;

            while *continue_listening_clone.lock().unwrap() {
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
                                                // Normal, building packet
                                            }
                                            _ => {
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
                    }
                }
            }

            *continue_listening_clone.lock().unwrap() = false;
        });

        FlemRx {
            rx_listener_handle: rx_thread_handle,
            rx_packet_queue: rx,
        }
    }

    pub fn unlisten(&mut self) {
        *self.continue_listening.lock().unwrap() = false;
    }

    pub fn send(&mut self, packet: &flem::Packet<T>) -> Option<()> {
        if let Some(mutex_ref) = self.tx_port.as_ref() {
            if let Ok(mut port) = mutex_ref.lock() {
                if let Ok(_) = port.as_mut().write_all(&packet.bytes()) {
                    port.as_mut().flush().unwrap();
                    return Some(());
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            return None;
        }

        // if let Ok(()) = self
        //     .tx_port
        //     .as_ref()
        //     .unwrap()
        //     .lock()
        //     .unwrap()
        //     .as_mut()
        //     .flush()
        // {
        //     return Some(());
        // } else {
        //     return None;
        // }
    }
}

#[cfg(test)]
mod tests {
    use crate::FlemSerial;
    use std::{
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    #[test]
    fn test_list_serial_ports() {
        let mut flem_serial = FlemSerial::<512>::new();

        let ports = flem_serial.list_serial_ports().unwrap();
        print!("{:?}", ports);
        let result = flem_serial.connect(&ports[4], 115200);
        match result {
            Ok(()) => {
                let flem_rx = flem_serial.listen();

                // let listener_handle = thread::spawn(move || {
                //     // Handle the incoming packets
                //     flem_serial.get_received_packets().iter()
                // });

                for i in 0..100 {
                    thread::sleep(Duration::from_millis(10));
                }

                let mut valid_packets = 0;
                let rx_packet_queue = flem_rx.rx_packet_queue;
                loop {
                    match rx_packet_queue.recv() {
                        Ok(packet) => {
                            let x = packet.get_request();
                            valid_packets += 1;
                        }
                        Err(error) => {
                            let error = error;
                            println!("{}", error);
                        }
                    }
                }

                flem_serial.unlisten();

                flem_rx.rx_listener_handle.join().unwrap();
            }
            Err(error) => {}
        }
    }
}
