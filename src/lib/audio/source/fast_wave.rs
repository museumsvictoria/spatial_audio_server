use std::sync::mpsc;
use hound::{self, SampleFormat, WavSpec};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::fs::File;
use fxhash::FxHashMap;
use nannou::audio::sample::Sample;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub struct BufferID(u64);

const NUM_BUFFERS: usize = 200;
const STEPS_AHEAD: usize = 2;

enum BufferMsg {
    ID(BufferID),
    Spec(WavSpec),
    Len(u32),
    Duration(u32),
}

pub enum FastWavesCommand{
    NewBuffer(Buffer),
    Destroy(BufferID),
    Spec(BufferID),
    Len(BufferID),
    Duration(BufferID),
    Fill(BufferID),
    Seek(BufferID, u32),
}

struct Buffer{
    pub reader: hound::WavReader<BufReader<File>>,
    pub reader_tx: mpsc::SyncSender<VecDeque<f32>>,
    pub info_tx: mpsc::Sender<BufferMsg>,
    pub buffer_size: usize,
    pub kill_rx: mpsc::Receiver<bool>,
}

struct FastWaves{
    pub buffer_count: u64,
    pub fast_waves_rx: mpsc::Receiver<FastWavesCommand>,
    pub buffers: FxHashMap<BufferID, Buffer>,
}


pub struct FastWave{
    buffer_id: BufferID,
    fast_waves_tx: mpsc::Sender<FastWavesCommand>,
    reader_rx: mpsc::Receiver<VecDeque<f32>>,
    info_rx: mpsc::Receiver<BufferMsg>,
    kill_tx: mpsc::Sender<bool>,
    length: u32,
    duration: u32,
    buffer_left: usize,
    buffer_size: usize,
    min_buffer: usize,
    batch: VecDeque<f32>,
}


impl FastWave{
    pub fn from_path<P>(path: P, fast_waves_tx: mpsc::Sender<FastWavesCommand>) -> Result<Self, hound::Error>
        where
        P: AsRef<Path>,
        {
            let reader = hound::WavReader::open(path)?;
            let spec = reader.spec();
            let length = reader.len();
            let duration = reader.duration();
            let channels = spec.channels as usize;
            let buffer_size = super::super::FRAMES_PER_BUFFER * channels * NUM_BUFFERS;
            let (reader_tx, reader_rx) = mpsc::sync_channel::<VecDeque<f32>>(buffer_size * STEPS_AHEAD);
            let (info_tx, info_rx) = mpsc::channel::<BufferMsg>();
            let (kill_tx, kill_rx) = mpsc::channel::<bool>();
            fast_waves_tx.send(FastWavesCommand::NewBuffer(Buffer{ reader, reader_tx, 
                info_tx, buffer_size, kill_rx }));
            let buffer_id = match info_rx.recv().expect("didn't recv buffer id on new buffer"){
                BufferMsg::ID(id) => id,
                _ => panic!("received wrong buffer message"), 
            };
            Ok(FastWave{ buffer_id, fast_waves_tx, reader_rx, info_rx, kill_tx, length, duration,
                buffer_left: buffer_size, buffer_size, min_buffer: (buffer_size as f64 * 0.75) as usize,
                batch: VecDeque::new()
            })
        }

    pub fn spec(&self) -> WavSpec {
        //self.reader.spec()
        self.fast_waves_tx.send(FastWavesCommand::Spec(self.buffer_id));
        match self.info_rx.recv().expect("error receiving spec message"){
            BufferMsg::Spec(s) => s,
            _ => panic!("error receiving spec message, wrong type"),
        }
    }

    pub fn duration(&self) -> u32 {
        //self.reader.duration()
        /*
        self.fast_waves_tx.send(FastWavesCommand::Duration(self.buffer_id));
        match self.info_rx.recv().expect("error receiving spec message"){
            BufferMsg::Duration(d) => d,
            _ => panic!("error receiving spec message, wrong type"),
        }
        */
        self.duration
    }

    pub fn seek(&mut self, time: u32) -> io::Result<()> {
        //self.reader.seek(time)
        self.fast_waves_tx.send(FastWavesCommand::Seek(self.buffer_id, time));
        // Ignoring for now
        Ok(())
    }

    pub fn len(&self) -> u32 {
        //self.reader.len()
        /*
        self.fast_waves_tx.send(FastWavesCommand::Len(self.buffer_id));
        match self.info_rx.recv().expect("error receiving spec message"){
            BufferMsg::Len(l) => l,
            _ => panic!("error receiving spec message, wrong type"),
        }
        */
        self.length
    }

    pub fn sample(&mut self) -> Option<f32>{
        self.buffer_left -= 1;
        if self.buffer_left < self.min_buffer {
            self.fast_waves_tx.send(FastWavesCommand::Fill(self.buffer_id));
            /*
            eprintln!("buf left: {}", self.buffer_left);
            eprintln!("min buf: {}", self.min_buffer);
            eprintln!("buf size: {}", self.buffer_size);
            */
            self.buffer_left += self.buffer_size;
            //eprintln!("buf left after: {}", self.buffer_left);
        }
        if self.batch.len() <= 0 {
            self.get_batch()
        }
        self.batch.pop_front()
            
    }

    fn get_batch(&mut self){
        match self.reader_rx.try_recv() {
            Ok(s) => self.batch = s,
            Err(e) => {
                eprintln!("Error receiving sample {:?}", e);
            },
        }
    }
}

// Clean up in other thread
impl Drop for FastWave {
    fn drop(&mut self) {
        self.fast_waves_tx.send(FastWavesCommand::Destroy(self.buffer_id));
        self.kill_tx.send(true);
    }
}

impl FastWaves{
    fn fill(&mut self, id: BufferID, steps_ahead: usize){
        self.buffers.get_mut(&id).map(|b| {
            let spec = b.reader.spec();
            let buffer_size = b.buffer_size * steps_ahead;
            let batch_length = buffer_size / 10;
            macro_rules! next_sample {
                ($T:ty) => {{
                    let mut samples_batch = VecDeque::with_capacity(batch_length);
                    for i in 0..batch_length{
                        match b.reader.samples::<$T>().next() {
                                Some(Err(err)) => {
                                    eprintln!("failed to read sample: {}", err);
                                },
                                Some(Ok(sample)) => {
                                    samples_batch.push_back(sample.to_sample::<f32>());
                                },
                                None => (),
                            }
                        
                    }
                    if b.kill_rx.try_recv().is_ok() { 
                        println!("killed");
                        return (); 
                    }
                    match b.reader_tx.try_send(samples_batch){
                        Ok(_) => (),
                        Err(e) => {
                            eprintln!("Error sending sample {:?}", e);
                            ()
                        },
                    }
                }};
            }
            let amount = buffer_size / batch_length;
            for i in 0..amount{
                match (spec.sample_format, spec.bits_per_sample) {
                    (SampleFormat::Float, 32) => next_sample!(f32),
                    (SampleFormat::Int, 8) => next_sample!(i8),
                    (SampleFormat::Int, 16) => next_sample!(i16),
                    (SampleFormat::Int, 32) => next_sample!(i32),
                    _ => {
                        eprintln!(
                            "Unsupported bit depth {} - currently only 8, 16 and 32 are supported",
                            spec.bits_per_sample
                            );
                    },
                }
            }
        });
    }
}

pub fn run(fast_waves_rx: mpsc::Receiver<FastWavesCommand>){
    let mut fs = FastWaves{ buffer_count: 0 as u64, fast_waves_rx, buffers: FxHashMap::default() };
    loop {
        match fs.fast_waves_rx.recv() {
            Ok(FastWavesCommand::NewBuffer(b)) => {
                let buffer_id = BufferID(fs.buffer_count);
                b.info_tx.send(BufferMsg::ID(buffer_id));
                fs.buffers.insert(buffer_id, b);
                fs.buffer_count += 1;

                fs.fill(buffer_id, 1);
            },
            Ok(FastWavesCommand::Destroy(id)) => {
                fs.buffers.remove(&id);
            },
            Ok(FastWavesCommand::Spec(id)) => {
                fs.buffers.get(&id).map(|b| {
                    b.info_tx.send(BufferMsg::Spec(b.reader.spec()))
                });
            },
            Ok(FastWavesCommand::Len(id)) => {
                fs.buffers.get(&id).map(|b| {
                    b.info_tx.send(BufferMsg::Len(b.reader.len()))
                });
            },
            Ok(FastWavesCommand::Duration(id)) => {
                fs.buffers.get(&id).map(|b| {
                    b.info_tx.send(BufferMsg::Duration(b.reader.duration()))
                });
            },
            Ok(FastWavesCommand::Seek(id, time)) => {
                fs.buffers.get_mut(&id).map(|b| {
                        b.reader.seek(time)
                });
            },
            Ok(FastWavesCommand::Fill(id)) => {
                fs.fill(id, 1);
            }
            Err(e) => eprintln!("error receiving fast waves commands {}", e),

        }
    }
}
