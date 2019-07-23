use std::collections::HashMap;
use std::error::Error;

use rand::{Rng, SeedableRng};
use serde_json::json;
use smart_contract_macros::smart_contract;

use smart_contract::log;
use smart_contract::payload::Parameters;
use smart_contract::transaction::{Transaction, Transfer};

static mut COUNTER: u32 = 0;

fn generate_id() -> String {
    unsafe {
        COUNTER = COUNTER + 1;
        COUNTER.to_string()
    }
}

fn to_hex_string(bytes: [u8; 32]) -> String {
    let strs: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    strs.join("")
}

fn random(params: &Parameters) -> u32 {
    let mut seed: [u8; 16] = [0; 16];

    for (rref, val) in seed.iter_mut().zip(params.round_id.iter().zip(&params.transaction_id).map(|(a, b)| a + b)) {
        *rref = val;
    };
    let mut rng = rand::rngs::SmallRng::from_seed(seed);

    let num = rng.gen_range(0, 100);

    return num;
}

#[derive(Debug, Clone)]
struct Player {
    sender: [u8; 32],
    tx_id: [u8; 32],
    stake: u64,
    vote: u8,
}

#[derive(Debug, Clone)]
struct Match {
    id: String,
    p1: Player,
    p2: Option<Player>,

    p1_payout: i64,
    p2_payout: i64,
    total_pot_paid: i64,
}

impl Match {
    pub fn new(id: String, player: Player) -> Match {
        let m = Match {
            id: id,
            p1: player,
            p2: None,
            p1_payout: 0,
            p2_payout: 0,
            total_pot_paid: 0,
        };

        return m;
    }

    pub fn play(&mut self, p2: Player, pot: u64) {
        if self.p1.vote == 2 && p2.vote == 2 {
            // Both players lose the stakes. The stakes go to the pot

            self.p1_payout = -(self.p1.stake as i64);
            self.p2_payout = -(p2.stake as i64);

            self.total_pot_paid = (self.p1.stake + p2.stake) as i64;
//            self.pot += p1.stake;
//            self.pot += p2.stake;
        } else if self.p1.vote == 1 && p2.vote == 1 {
            // Both players get back their stakes plus pot rewards

            let p1_pot_payout: i64 = (0.01 * pot as f64) as i64;
            self.p1_payout = self.p1.stake as i64 + p1_pot_payout;

            let p2_pot_payout: i64 = (0.01 * pot as f64) as i64;
            self.p2_payout = p2.stake as i64 + p2_pot_payout;

            self.total_pot_paid = -(p1_pot_payout + p2_pot_payout);
        } else if self.p1.vote == 1 && p2.vote == 2 {
            // Player  1 lose his stake

            self.p1_payout = -(self.p1.stake as i64);

            // Player 2 get back his stake, plus Player 1 stake and pot reward

            let p2_pot_payout: i64 = (0.015 * pot as f64) as i64;
            self.p2_payout = (p2.stake + self.p1.stake) as i64 + p2_pot_payout;

            self.total_pot_paid = -p2_pot_payout;
        } else if self.p1.vote == 2 && p2.vote == 1 {
            // Player 1 get back his stake, plus Player 2 stake and pot reward

            let p1_pot_payout: i64 = (0.015 * pot as f64) as i64;
            self.p1_payout = (p2.stake + self.p1.stake) as i64 + p1_pot_payout;

            self.total_pot_paid = -p1_pot_payout;

            // Player 2 lose his stake

            self.p2_payout = -(p2.stake as i64);
        }

        self.p2 = Some(p2);
    }
}

fn update_balance(balances: &mut HashMap<[u8; 32], u64>, sender: [u8; 32], amount: i64) {
    let recipient_balance = match balances.get(&sender) {
        Some(balance) => *balance,
        None => 0,
    };

    let mut updated = recipient_balance as i64 + amount;
    if updated.is_negative() {
        updated = 0
    }

    balances.insert(sender, updated as u64);
}

struct PrisonerDilemma {
    balances: HashMap<[u8; 32], u64>,
    pot: u64,
    threshold: u32,
    waiting: Vec<Match>,
    history: Vec<Match>,
}

#[smart_contract]
impl PrisonerDilemma {
    fn init(params: &mut Parameters) -> Self {
        let mut balances = HashMap::new();

        balances.insert(params.sender, 100_000);

        Self {
            balances: balances,
            threshold: 50,
            pot: 100_000,
            waiting: Vec::new(),
            history: Vec::new(),
        }
    }

    fn play(&mut self, params: &mut Parameters) -> Result<(), Box<dyn Error>> {
        let sender = params.sender;
        let tx_id = params.transaction_id;
        let amount: u64 = params.amount;

        let vote: u8 = params.read();

        if vote != 1 && vote != 2 {
            return Err("Vote must be either 1 (cooperate) or 2 (defect).".into());
        }

        let p = Player {
            sender: sender,
            tx_id: tx_id,
            stake: amount,
            vote: vote,
        };

        if random(params) > self.threshold {
            self.threshold += 1;

            let id = generate_id();
            self.waiting.push(Match::new(id.clone(), p));

            log(&format!("Created new match {}. Waiting...", id));
            return Ok(());
        }

        if self.threshold > 0 {
            self.threshold -= 1;
        }

        let i = match self.waiting.iter_mut().position(|m| m.p1.sender != sender) {
            Some(v) => v,
            None => {
                let id = generate_id();
                self.waiting.push(Match::new(id.clone(), p));

                log(&format!("There's no waiting player. Created new match {}. Waiting...", id));

                return Ok(());
            }
        };

        let mut m = self.waiting.get(i).unwrap().clone();

        self.waiting.remove(i);

        m.play(p, self.pot);

        // Update the players' balances

        let p2 = m.p2.clone().unwrap();

        update_balance(&mut self.balances, p2.sender, m.p2_payout);
        update_balance(&mut self.balances, m.p1.sender, m.p1_payout);

        // Update the pot.
        let mut new_pot: i64 = self.pot as i64 + m.total_pot_paid;
        if new_pot < 0 {
            new_pot = 0;
        }
        self.pot = new_pot as u64;

        // Save the match
        self.history.push(m.clone());

        // TODO prune the history

        Ok(())
    }

    fn result(&mut self, params: &mut Parameters) -> Result<(), Box<dyn Error>> {
        let id: String = params.read();

        if self.waiting.iter().find(|m| m.id == id).is_some() {
            return Err("Your match is still waiting for other player.".into());
        }

        let found = match self.history.iter().find(|m| m.id == id) {
            Some(m) => m,
            None => {
//                log(&format!("The match {} does not exist.", id));
                return Err("The match does not exist.".into());
            }
        };

        let p2 = found.p2.clone().unwrap();

//        log(&format!("Match {}, Payout {}: {}, Payout {}: {}", found.id, to_hex_string(found.p1.sender), found.p1_payout, to_hex_string(p2.sender), found.p2_payout));

        let result = json!({
            "player_1": json!({
                            "sender": to_hex_string(found.p1.sender),
                            "payout": found.p1_payout,
                        }),
            "player_2": json!({
                            "sender": to_hex_string(p2.sender),
                            "payout": found.p2_payout,
                        }),
        });
        log(&result.to_string());

        Ok(())
    }

    fn get_balance(&mut self, params: &mut Parameters) -> Result<(), Box<dyn Error>> {
        let sender_balance = match self.balances.get(&params.sender) {
            Some(balance) => *balance,
            None => 0,
        };

        log(&sender_balance.to_string());

        Ok(())
    }

    fn cash_out(&mut self, params: &mut Parameters) -> Result<(), Box<dyn Error>> {
        let sender_balance = match self.balances.get(&params.sender) {
            Some(balance) => *balance,
            None => 0,
        };
        if sender_balance == 0 {
            return Err("Sender has no PERLS".into());
        }

        Transfer {
            destination: params.sender,
            amount: sender_balance,
            func_name: vec![],
            func_params: vec![],
        }.send_transaction();

        self.balances.insert(params.sender, 0);

        Ok(())
    }
}