use std::sync::mpsc;
use hound::{self, SampleFormat, WavSpec};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::fs::File;
use fxhash::FxHashMap;
use nannou::audio::sample::Sample;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
struct BufferID(u64);

const NUM_BUFFERS: usize = 3;

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
}

struct Buffer{
    pub reader: hound::WavReader<BufReader<File>>,
    pub reader_tx: mpsc::SyncSender<f32>,
    pub info_tx: mpsc::Sender<BufferMsg>,
}

struct FastWaves{
    pub buffer_count: u64,
    pub fast_waves_rx: mpsc::Receiver<FastWavesCommand>,
    pub buffers: FxHashMap<BufferID, Buffer>,
}


pub struct FastWave{
    buffer_id: BufferID,
    fast_waves_tx: mpsc::Sender<FastWavesCommand>,
    reader_rx: mpsc::Receiver<f32>,
    info_rx: mpsc::Receiver<BufferMsg>,
}


impl FastWave{
    pub fn from_path<P>(path: P, fast_waves_tx: mpsc::Sender<FastWavesCommand>) -> Result<Self, hound::Error>
        where
        P: AsRef<Path>,
        {
            let reader = hound::WavReader::open(path)?;
            let spec = reader.spec();
            let channels = spec.channels as usize;
            let num_samples = super::super::FRAMES_PER_BUFFER * channels * NUM_BUFFERS;
            let (reader_tx, reader_rx) = mpsc::sync_channel::<f32>(num_samples*2);
            let (info_tx, info_rx) = mpsc::channel::<BufferMsg>();
            fast_waves_tx.send(FastWavesCommand::NewBuffer(Buffer{reader, reader_tx, info_tx}));
            let buffer_id = match info_rx.recv().expect("didn't recv buffer id on new buffer"){
                BufferMsg::ID(id) => id,
                _ => panic!("received wrong buffer message"), 
            };
            Ok(FastWave{ buffer_id, fast_waves_tx, reader_rx, info_rx })
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
        self.fast_waves_tx.send(FastWavesCommand::Duration(self.buffer_id));
        match self.info_rx.recv().expect("error receiving spec message"){
            BufferMsg::Duration(d) => d,
            _ => panic!("error receiving spec message, wrong type"),
        }
    }

    pub fn seek(&mut self, time: u32) -> io::Result<()> {
        //self.reader.seek(time)
        // Ignoring for now
        Ok(())
    }

    pub fn len(&self) -> u32 {
        //self.reader.len()
        self.fast_waves_tx.send(FastWavesCommand::Len(self.buffer_id));
        match self.info_rx.recv().expect("error receiving spec message"){
            BufferMsg::Len(l) => l,
            _ => panic!("error receiving spec message, wrong type"),
        }
    }

    /*
    pub fn samples<'wr, S: hound::Sample>(&'wr mut self) -> WavSamples<'wr, BufReader<File>, S>{
        self.reader.samples::<S>()
    }
    */
    pub fn sample(&mut self) -> Option<f32>{
        // TODO this might be too slow
        self.fast_waves_tx.send(FastWavesCommand::Fill(self.buffer_id));
        match self.reader_rx.try_recv() {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("Error receiving sample {:?}", e);
                None
            },
        }
    }
}

// Clean up in other thread
impl Drop for FastWave {
    fn drop(&mut self) {
        self.fast_waves_tx.send(FastWavesCommand::Destroy(self.buffer_id));
    }
}

impl FastWaves{
    fn fill(&mut self, id: BufferID, num_buf: usize){
        self.buffers.get_mut(&id).map(|b| {
            let spec = b.reader.spec();
            let channels = spec.channels as usize;
            let num_samples = super::super::FRAMES_PER_BUFFER * channels;
            macro_rules! next_sample {
                ($T:ty) => {{
                    match b.reader.samples::<$T>().next() {
                        Some(Err(err)) => {
                            eprintln!("failed to read sample: {}", err);
                        },
                        Some(Ok(sample)) => {
                            //self.sample_index += 1;
                            match b.reader_tx.try_send(sample.to_sample::<f32>()) {
                                Ok(_) => (),
                                Err(e) => {
                                    eprintln!("Error receiving sample {:?}", e);
                                    ()
                                },
                            }
                        },
                        None => (),
                    }
                }};
            }
            for i in 0..num_samples {
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

                fs.fill(buffer_id, NUM_BUFFERS);
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
            Ok(FastWavesCommand::Fill(id)) => {
                fs.fill(id, NUM_BUFFERS);
            }
            Err(e) => eprintln!("error receiving fast waves commands {}", e),

        }
    }
}
