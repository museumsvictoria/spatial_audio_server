use std::sync::mpsc;
use hound::{self, SampleFormat, WavSpec};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::fs::File;

struct Buffer{
    reader: hound::WavReader<BufReader<File>>,
    reader_tx: mpsc::Sender<f32>,
}

struct FastWaves{
    fast_waves_rx: mpsc::Receiver<FastWavesCommand>,
    buffers: Vec<Buffer>,
}

pub enum FastWavesCommand{
    NewBuffer(Buffer),
}

pub struct FastWave{
    fast_waves_tx: mpsc::Sender<FastWavesCommand>,
    reader_rx: mpsc::Receiver<f32>,
}



impl FastWave{
    pub fn from_path<P>(path: P, fast_waves_tx: mpsc::Sender<FastWavesCommand>) -> Result<Self, hound::Error>
        where
        P: AsRef<Path>,
        {
            let reader = hound::WavReader::open(path)?;
            let (reader_tx, reader_rx) = mpsc::channel::<f32>();
            fast_waves_tx.send(FastWavesCommand::NewBuffer(Buffer{reader, reader_tx}));
            Ok(FastWave{ fast_waves_tx, reader_rx })
        }

    pub fn spec(&self) -> WavSpec {
        //self.reader.spec()
        unimplemented!()
    }

    pub fn duration(&self) -> u32 {
        //self.reader.duration()
        unimplemented!()
    }

    pub fn seek(&mut self, time: u32) -> io::Result<()> {
        //self.reader.seek(time)
        unimplemented!()
    }

    pub fn len(&self) -> u32 {
        //self.reader.len()
        unimplemented!()
    }

    fn fill_ahead<S: hound::Sample>(&mut self, amount: usize){
        unimplemented!();
        /*
        let test_it = self.reader.samples::<S>();
        for i in 0..amount {
            match test_it.next() {
                Some(Err(err)) => {
                    eprintln!("failed to read sample: {}", err);
                },
                Some(Ok(sample)) => {
                    //self.sample_index += 1;
                    self.sample_tx.send(sample.to_sample::<f32>());
                },
                None => (),
            }
        }
        */
    }
    /*
    pub fn samples<'wr, S: hound::Sample>(&'wr mut self) -> WavSamples<'wr, BufReader<File>, S>{
        self.reader.samples::<S>()
    }
    */
    pub fn samples<'wr, S: hound::Sample>(&'wr mut self) -> FastSamples<'wr>{
        // TODO this might be too slow
        unimplemented!();
        //FastSamples{ fs: &mut self }
    }
}

pub struct FastSamples<'wr>{
    fs: &'wr mut FastWave,
}

impl<'wr> Iterator for FastSamples<'wr>{
    type Item = Result<f32, mpsc::TryRecvError>;

    fn next(&mut self) -> Option<Result<f32, mpsc::TryRecvError>> {
        match self.fs.reader_rx.try_recv() {
            Ok(s) => Some(Ok(s)),
            Err(e) => Some(Err(e)),
        }
    }
}

pub fn run(fast_waves_rx: mpsc::Receiver<FastWavesCommand>){
}
