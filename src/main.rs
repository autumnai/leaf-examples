extern crate rustc_serialize;
extern crate docopt;
extern crate csv;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate leaf;
extern crate collenchyma as co;

use std::io::prelude::*;
use std::fs::File;
use std::sync::{Arc, RwLock};

use hyper::Client;

use docopt::Docopt;
use csv::{Reader};
use leaf::layer::*;
use leaf::layers::*;
use leaf::solver::*;
use leaf::util::*;
use co::prelude::*;

const MAIN_USAGE: &'static str = "
Leaf Examples

Usage:
    leaf-examples load-dataset <dataset-name>
    leaf-examples mnist <model-name> [--batch-size <batch-size>] [--learning-rate <learning-rate>] [--momentum <momentum>]
    leaf-examples (-h | --help)
    leaf-examples --version

Options:
    <model-name>            Which MNIST model to use. Valid values: [linear, mlp, conv]

    -h --help               Show this screen.
    --version               Show version.
";

#[derive(Debug, RustcDecodable)]
struct MainArgs {
    arg_dataset_name: Option<String>,
    arg_model_name: Option<String>,
    arg_batch_size: Option<usize>,
    arg_learning_rate: Option<f32>,
    arg_momentum: Option<f32>,
    cmd_load_dataset: bool,
    cmd_mnist: bool,
}

#[cfg(not(test))]
#[allow(unused_must_use)]
fn main() {
    env_logger::init().unwrap();
    // Parse Arguments
    let args: MainArgs = Docopt::new(MAIN_USAGE)
                            .and_then(|d| d.decode())
                            .unwrap_or_else(|e| e.exit());

    if args.cmd_load_dataset {
        match &*args.arg_dataset_name.unwrap() {
            "mnist" => {
                let datasets = ["mnist_test.csv", "mnist_train.csv"];
                for (i, v) in datasets.iter().enumerate() {
                    println!("Downloading... {}/{}: {}", i+1, datasets.len(), v);
                    let mut body = String::new();

                    Client::new()
                        .get(&format!("http://pjreddie.com/media/files/{}", v))
                        .send().unwrap().read_to_string(&mut body);

                    File::create(format!("assets/{}", v))
                        .unwrap()
                        .write_all(&body.into_bytes());
                }
                println!("{}", "MNIST dataset downloaded".to_string())
            },
            _ => println!("{}", "Failed to download MNIST dataset!".to_string())
        }
    } else if args.cmd_mnist {
        run_mnist(args.arg_model_name, args.arg_batch_size, args.arg_learning_rate, args.arg_momentum);
    }
}

#[allow(dead_code)]
fn run_mnist(model_name: Option<String>, batch_size: Option<usize>, learning_rate: Option<f32>, momentum: Option<f32>) {
    let mut rdr = Reader::from_file("assets/mnist_train.csv").unwrap();
    let mut decoded_images = rdr.decode().map(|row|
        match row {
            Ok(value) => {
                let row_vec: Box<Vec<u8>> = Box::new(value);
                let label = row_vec[0];
                let mut pixels = vec![0u8; 784];
                for (place, element) in pixels.iter_mut().zip(row_vec.iter().skip(1)) {
                    *place = *element;
                }
                // TODO: reintroduce Cuticula
                // let img = Image::from_luma_pixels(28, 28, pixels);
                // match img {
                //     Ok(in_img) => {
                //         println!("({}): {:?}", label, in_img.transform(vec![28, 28]));
                //     },
                //     Err(_) => unimplemented!()
                // }
                (label, pixels)
            },
            _ => {
                println!("no value");
                panic!();
            }
        }
    );

    let batch_size = batch_size.unwrap_or(1);
    let learning_rate = learning_rate.unwrap_or(0.001f32);
    let momentum = momentum.unwrap_or(0f32);

    let mut net_cfg = SequentialConfig::default();
    net_cfg.add_input("data", &vec![batch_size, 28, 28]);
    net_cfg.force_backward = true;

    match &*model_name.unwrap_or("none".to_owned()) {
        "conv" => {
            let reshape_cfg = LayerConfig::new("reshape", ReshapeConfig::of_shape(&vec![batch_size, 1, 28, 28]));
            net_cfg.add_layer(reshape_cfg);
            let conv_cfg = ConvolutionConfig { num_output: 20, filter_shape: vec![5], stride: vec![1], padding: vec![0] };
            net_cfg.add_layer(LayerConfig::new("conv", conv_cfg));
            let pool_cfg = PoolingConfig { mode: PoolingMode::Max, filter_shape: vec![2], stride: vec![2], padding: vec![0] };
            net_cfg.add_layer(LayerConfig::new("pooling", pool_cfg));
            net_cfg.add_layer(LayerConfig::new("linear1", LinearConfig { output_size: 500 }));
            net_cfg.add_layer(LayerConfig::new("sigmoid", LayerType::Sigmoid));
            net_cfg.add_layer(LayerConfig::new("linear2", LinearConfig { output_size: 10 }));
        },
        "mlp" => {
            let reshape_cfg = LayerConfig::new("reshape", LayerType::Reshape(ReshapeConfig::of_shape(&vec![batch_size, 784])));
            net_cfg.add_layer(reshape_cfg);
            net_cfg.add_layer(LayerConfig::new("linear1", LayerType::Linear(LinearConfig { output_size: 1568 })));
            net_cfg.add_layer(LayerConfig::new("sigmoid", LayerType::Sigmoid));
            net_cfg.add_layer(LayerConfig::new("linear2", LayerType::Linear(LinearConfig { output_size: 10 })));
        },
        "linear" => {
            net_cfg.add_layer(LayerConfig::new("linear", LayerType::Linear(LinearConfig { output_size: 10 })));
        }
        _ => { panic!("Unknown model. Try one of [linear, mlp, conv]")}
    }
    net_cfg.add_layer(LayerConfig::new("log_softmax", LayerType::LogSoftmax));

    let mut classifier_cfg = SequentialConfig::default();
    classifier_cfg.add_input("network_out", &vec![batch_size, 10]);
    classifier_cfg.add_input("label", &vec![batch_size, 1]);
    // set up nll loss
    let nll_layer_cfg = NegativeLogLikelihoodConfig { num_classes: 10 };
    let nll_cfg = LayerConfig::new("nll", LayerType::NegativeLogLikelihood(nll_layer_cfg));
    classifier_cfg.add_layer(nll_cfg);

    // set up backends
    let backend = ::std::rc::Rc::new(Backend::<Cuda>::default().unwrap());

    // set up solver
    let mut solver_cfg = SolverConfig { minibatch_size: batch_size, base_lr: learning_rate, momentum: momentum, .. SolverConfig::default() };
    solver_cfg.network = LayerConfig::new("network", net_cfg);
    solver_cfg.objective = LayerConfig::new("classifier", classifier_cfg);
    let mut solver = Solver::from_config(backend.clone(), backend.clone(), &solver_cfg);

    // set up confusion matrix
    let mut confusion = ::leaf::solver::ConfusionMatrix::new(10);
    confusion.set_capacity(Some(1000));

    let inp = SharedTensor::<f32>::new(&[batch_size, 1, 28, 28]);
    let label = SharedTensor::<f32>::new(&[batch_size, 1]);

    let inp_lock = Arc::new(RwLock::new(inp));
    let label_lock = Arc::new(RwLock::new(label));

    for _ in 0..(60000 / batch_size) {
        // write input
        let mut targets = Vec::new();
        for (batch_n, (label_val, input)) in decoded_images.by_ref().take(batch_size).enumerate() {
            let mut inp = inp_lock.write().unwrap();
            let mut label = label_lock.write().unwrap();
            write_batch_sample(&mut inp, &input, batch_n);
            write_batch_sample(&mut label, &[label_val], batch_n);

            targets.push(label_val as usize);
        }
        // train the network!
        let infered_out = solver.train_minibatch(inp_lock.clone(), label_lock.clone());

        let mut infered = infered_out.write().unwrap();
        let predictions = confusion.get_predictions(&mut infered);

        confusion.add_samples(&predictions, &targets);
        println!("Last sample: {} | Accuracy {}", confusion.samples().iter().last().unwrap(), confusion.accuracy());
    }
}
