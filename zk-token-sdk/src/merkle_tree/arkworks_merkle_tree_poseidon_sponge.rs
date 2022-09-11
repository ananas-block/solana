#![allow(unused)] // temporary
#![allow(clippy::needless_range_loop)]

use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, Read, SerializationError, Write};
use ark_std::vec::Vec;
use ark_ff::bytes::{ToBytes, FromBytes};
use ark_sponge;
use ark_sponge::poseidon::PoseidonSponge;
use ark_ff::Fp256;
use ark_ff::BigInteger256;
use ark_ed_on_bn254;
use ark_ed_on_bn254::Fq;

use ark_sponge::CryptographicSponge;
use crate::poseidon_params::get_params;
use arkworks_gadgets::utils::{
	get_mds_poseidon_circom_bn254_x5_3, get_rounds_poseidon_circom_bn254_x5_3, parse_vec,
};
use arkworks_gadgets::poseidon::{PoseidonParameters, PoseidonError, Rounds,circom::CircomCRH, sbox::PoseidonSbox};
use ark_crypto_primitives::{crh::{TwoToOneCRH, CRH}, Error};

#[derive(Default, Clone)]
pub struct PoseidonCircomRounds3;

impl Rounds for PoseidonCircomRounds3 {
    const FULL_ROUNDS: usize = 8;
    const PARTIAL_ROUNDS: usize = 57;
    const SBOX: PoseidonSbox = PoseidonSbox::Exponentiation(5);
    const WIDTH: usize = 3;
}
pub type PoseidonCircomCRH3 = CircomCRH<Fq, PoseidonCircomRounds3>;

pub fn hash_64_to_vec(input_bytes: Vec<u8>) -> [u8; 32] {

    let rounds = get_rounds_poseidon_circom_bn254_x5_3::<Fq>();
    let mds = get_mds_poseidon_circom_bn254_x5_3::<Fq>();
    let params = PoseidonParameters::<Fq>::new(rounds, mds);
    //perform the test 1000x

    //generating random test input
    //let mut rng = test_rng();
    println!("Bytes: {:?}", input_bytes);
    //println!("Bytes: {:?}", input_bytes[32..64]);
    //generating reference poseidon hash with library to test against
	let poseidon_res =
		<PoseidonCircomCRH3 as TwoToOneCRH>::evaluate(&params, &input_bytes[0..32], &input_bytes[32..64])
			.unwrap();

    //parsing reference hash to bytes
    let mut hash_bytes = [0u8;32];
    <Fq as ToBytes>::write(&poseidon_res, &mut hash_bytes[..]);

    hash_bytes
}
//pub type TwoToOneDigest<P> = <<P as Config>::TwoToOneHash as TwoToOneCRH>::Output;
//pub type LeafDigest<P> = <<P as Config>::LeafHash as CRH>::Output;
//pub type TwoToOneParam<P> = <<P as Config>::TwoToOneHash as TwoToOneCRH>::Parameters;
//pub type LeafParam<P> = <<P as Config>::LeafHash as CRH>::Parameters;

/// Stores the hashes of a particular path (in order) from root to leaf.
/// For example:
/// ```tree_diagram
///         [A]
///        /   \
///      [B]    C
///     / \   /  \
///    D [E] F    H
///   .. / \ ....
///    [I] J
/// ```
///  Suppose we want to prove I, then `leaf_sibling_hash` is J, `auth_path` is `[C,D]`

#[derive(Clone, CanonicalSerialize, CanonicalDeserialize, Debug)]
pub struct Path {
    pub leaf_sibling_hash: Vec<u8>,
    /// The sibling of path node ordered from higher layer to lower layer (does not include root node).
    pub auth_path: Vec<Vec<u8>>,
    /// stores the leaf index of the node
    pub leaf_index: usize,
}

impl Path {
    /// The position of on_path node in `leaf_and_sibling_hash` and `non_leaf_and_sibling_hash_path`.
    /// `position[i]` is 0 (false) iff `i`th on-path node from top to bottom is on the left.
    ///
    /// This function simply converts `self.leaf_index` to boolean array in big endian form.
    fn position_list(&'_ self) -> impl '_ + Iterator<Item = bool> {
        (0..self.auth_path.len() + 1)
            .map(move |i| ((self.leaf_index >> i) & 1) != 0)
            .rev()
    }
}

/// Convert `computed_hash` and `sibling_hash` to bytes. `index` is the first `path.len()` bits of
/// the position of tree.
///
/// If the least significant bit of `index` is 0, then `input_1` will be left and `input_2` will be right.
/// Otherwise, `input_1` will be right and `input_2` will be left.
///
/// Returns: (left, right)
fn select_left_right_bytes<B: ToBytes>(
    index: usize,
    computed_hash: &B,
    sibling_hash: &B,
) -> Result<(Vec<u8>, Vec<u8>), crate::Error> {
    let is_left = index & 1 == 0;
    let mut left_bytes = ark_ff::to_bytes!(computed_hash)?;
    let mut right_bytes = ark_ff::to_bytes!(sibling_hash)?;
    if !is_left {
        core::mem::swap(&mut left_bytes, &mut right_bytes);
    }
    Ok((left_bytes, right_bytes))
}

fn select_left_right_bytes_transform<B: ToBytes>(
    index: usize,
    computed_hash: &B,
    sibling_hash: &B,
) -> Result<(Vec<u8>, Vec<u8>, bool), crate::Error> {
    let is_left = index & 1 == 0;
    let mut left_bytes = ark_ff::to_bytes!(computed_hash)?;
    let mut right_bytes = ark_ff::to_bytes!(sibling_hash)?;
    if !is_left {
        core::mem::swap(&mut left_bytes, &mut right_bytes);
        return Ok((left_bytes, right_bytes, true));
    }
    Ok((left_bytes, right_bytes, false))
}

impl Path {
    /// Verify that a leaf is at `self.index` of the merkle tree.
    /// * `leaf_size`: leaf size in number of bytes
    ///
    /// `verify` infers the tree height by setting `tree_height = self.auth_path.len() + 2`

    pub fn verify(
        &self,
        root_hash: &Vec<u8>,
        claimed_leaf_hash: &Vec<u8>,
    ) -> Result<bool, crate::Error> {
        // calculate leaf hash assumed to be calculated already
        //let claimed_leaf_hash = P::LeafHash::evaluate(&leaf_hash_params, &ark_ff::to_bytes!(&leaf)?)?;
        // check hash along the path from bottom to root
        //println!("Self: {:?}", self.clone());
        let (left_bytes, right_bytes) =
            select_left_right_bytes(self.leaf_index, &claimed_leaf_hash, &&self.leaf_sibling_hash)?;
        //println!("left: {:?}", left_bytes);
        //println!("right: {:?}", right_bytes);

        let mut curr_path_node = hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
        //println!("curr_path_node: {:?}", curr_path_node);

        //    P::TwoToOneHash::evaluate(&two_to_one_hash_params, &left_bytes, &right_bytes)?;

        // we will use `index` variable to track the position of path
        let mut index = self.leaf_index;
        index >>= 1;

        // Check levels between leaf level and root
        for level in (0..self.auth_path.len()).rev() {
            // check if path node at this level is left or right
            let (left_bytes, right_bytes) =
                select_left_right_bytes(index, &curr_path_node, &self.auth_path[level])?;
            //println!("left: {:?}", left_bytes);
            //println!("right: {:?}", right_bytes);
            // update curr_path_node
            curr_path_node = hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
            index >>= 1;
            //println!("curr_path_node: {:?}", curr_path_node);
        }

        // check if final hash is root
        if &curr_path_node != root_hash {
            return Ok(false);
        }

        Ok(true)
    }

    pub fn convert_path_to_circuit_ready_merkle_proof(
        &self,
        root_hash: &Vec<u8>,
        claimed_leaf_hash: &Vec<u8>,
    ) -> Result<Vec<(bool, Vec<u8>)>, crate::Error> {
        // calculate leaf hash assumed to be calculated already
        //let claimed_leaf_hash = P::LeafHash::evaluate(&leaf_hash_params, &ark_ff::to_bytes!(&leaf)?)?;
        // check hash along the path from bottom to root
        //println!("Self: {:?}", self.clone())
        let mut res: Vec<(bool, Vec<u8>)> = Vec::new();

        let (left_bytes, right_bytes, sibling_is_left_bool) =
            select_left_right_bytes_transform(self.leaf_index, &claimed_leaf_hash, &&self.leaf_sibling_hash)?;
        //println!("left: {:?}", left_bytes);
        //println!("right: {:?}", right_bytes);
        res.push((sibling_is_left_bool, self.leaf_sibling_hash.clone()));
        let mut curr_path_node = hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
        //println!("curr_path_node: {:?}", curr_path_node);

        //    P::TwoToOneHash::evaluate(&two_to_one_hash_params, &left_bytes, &right_bytes)?;

        // we will use `index` variable to track the position of path
        let mut index = self.leaf_index;
        index >>= 1;



        // Check levels between leaf level and root
        for level in (0..self.auth_path.len()).rev() {
            // check if path node at this level is left or right
            let (left_bytes, right_bytes, sibling_is_left_bool) =
                select_left_right_bytes_transform(index, &curr_path_node, &self.auth_path[level])?;
            //println!("left: {:?}", left_bytes);
            //println!("right: {:?}", right_bytes);
            // update curr_path_node
            curr_path_node = hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
            index >>= 1;
            res.push((sibling_is_left_bool, self.auth_path[level].clone()));
            //println!("curr_path_node: {:?}", curr_path_node);
        }

        Ok(res)
    }
}

/// Defines a merkle tree data structure.
/// This merkle tree has runtime fixed height, and assumes number of leaves is 2^height.
///
/// TODO: add RFC-6962 compatible merkle tree in the future.
/// For this release, padding will not be supported because of security concerns: if the leaf hash and two to one hash uses same underlying
/// CRH, a malicious prover can prove a leaf while the actual node is an inner node. In the future, we can prefix leaf hashes in different layers to
/// solve the problem.
#[derive(Clone)]
pub struct MerkleTree {
    /// stores the non-leaf nodes in level order. The first element is the root node.
    /// The ith nodes (starting at 1st) children are at indices `2*i`, `2*i+1`
    non_leaf_nodes: Vec<Vec<u8>>,
    /// store the hash of leaf nodes from left to right
    leaf_nodes: Vec<Vec<u8>>,
    /// Store the two-to-one hash parameters
    //two_to_one_hash_param: TwoToOneParam<P>,
    /// Store the leaf hash parameters
    //leaf_hash_param: LeafParam<P>,
    /// Stores the height of the MerkleTree
    height: usize,
}

impl MerkleTree {
    /// Create an empty merkle tree such that all leaves are zero-filled.
    /// Consider using a sparse merkle tree if you need the tree to be low memory
    /*
    pub fn blank(
        leaf_hash_param: &LeafParam<P>,
        two_to_one_hash_param: &TwoToOneParam<P>,
        height: usize,
    ) -> Result<Self, crate::Error> {
        let leaf = vec![0u8; P::LeafHash::INPUT_SIZE_BITS / 8];
        let leaves = vec![leaf; 1 << (height - 1)];
        Self::new(leaf_hash_param, two_to_one_hash_param, &leaves)
    }
    */
    /// Returns a new merkle tree. `leaves.len()` should be power of two.
    pub fn new(
        //leaf_hash_param: &LeafParam<P>,
        //two_to_one_hash_param: &TwoToOneParam<P>,
        leaf_nodes: &Vec<Vec<u8>>,
    ) -> Result<Self, crate::Error> {
        let leaf_nodes_size = leaf_nodes.len(); // size of the leaf layer
        assert!(
            leaf_nodes_size.is_power_of_two(),
            "`leaves.len() should be power of two"
        );
        let non_leaf_nodes_size = leaf_nodes_size - 1;

        let tree_height = tree_height(leaf_nodes_size);
        let hash_of_empty = hash_64_to_vec([0u8;64].to_vec());
        /*
        let hash_of_empty: TwoToOneDigest<P> = P::TwoToOneHash::evaluate(
            two_to_one_hash_param,
            &vec![0u8; P::TwoToOneHash::LEFT_INPUT_SIZE_BITS / 8],
            &vec![0u8; P::TwoToOneHash::RIGHT_INPUT_SIZE_BITS / 8],
        )?;*/

        // initialize the merkle tree as array of nodes in level order
        let mut non_leaf_nodes: Vec<Vec<u8>> = (0..non_leaf_nodes_size)
            .map(|_| hash_of_empty.to_vec().clone())
            .collect();
        //let mut leaf_nodes = *leaves.clone();

        /*
        let mut leaf_nodes: Vec<LeafDigest<P>> = Vec::with_capacity(leaf_nodes_size);
        */
        // Compute the starting indices for each non-leaf level of the tree
        let mut index = 0;
        let mut level_indices = Vec::with_capacity(tree_height - 1);
        for _ in 0..(tree_height - 1) {
            level_indices.push(index);
            index = left_child(index);
        }
        /*
        // compute and store hash values for each leaf
        for leaf in leaves.iter() {
            leaf_nodes.push(P::LeafHash::evaluate(
                leaf_hash_param,
                &ark_ff::to_bytes!(leaf)?,
            )?)
        }
        */
        // compute the hash values for the non-leaf bottom layer
        {
            let start_index = level_indices.pop().unwrap();
            let upper_bound = left_child(start_index);
            for current_index in start_index..upper_bound {
                // `left_child(current_index)` and `right_child(current_index) returns the position of
                // leaf in the whole tree (represented as a list in level order). We need to shift it
                // by `-upper_bound` to get the index in `leaf_nodes` list.
                let left_leaf_index = left_child(current_index) - upper_bound;
                let right_leaf_index = right_child(current_index) - upper_bound;
                // compute hash
                let left_bytes = ark_ff::to_bytes!(&leaf_nodes[left_leaf_index])?;
                let right_bytes = ark_ff::to_bytes!(&leaf_nodes[right_leaf_index])?;
                non_leaf_nodes[current_index] =
                    hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
            }
        }

        // compute the hash values for nodes in every other layer in the tree
        level_indices.reverse();
        for &start_index in &level_indices {
            // The layer beginning `start_index` ends at `upper_bound` (exclusive).
            let upper_bound = left_child(start_index);
            for current_index in start_index..upper_bound {
                let left_index = left_child(current_index);
                let right_index = right_child(current_index);
                let left_bytes = ark_ff::to_bytes!(&non_leaf_nodes[left_index])?;
                let right_bytes = ark_ff::to_bytes!(&non_leaf_nodes[right_index])?;
                non_leaf_nodes[current_index] =
                    hash_64_to_vec([&left_bytes[..], &right_bytes[..]].concat()).to_vec();
            }
        }

        Ok(MerkleTree {
            leaf_nodes: leaf_nodes.clone().to_vec(),
            non_leaf_nodes,
            height: tree_height,
            //two_to_one_hash_param: two_to_one_hash_param.clone(),
            //leaf_hash_param: leaf_hash_param.clone(),
        })
    }

    /// Returns the root of the Merkle tree.
    pub fn root(&self) -> Vec<u8> {
        self.non_leaf_nodes[0].clone()
    }

    /// Returns the height of the Merkle tree.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns the authentication path from leaf at `index` to root.
    pub fn generate_proof(&self, index: usize) -> Result<Path, crate::Error> {
        // gather basic tree information
        let tree_height = tree_height(self.leaf_nodes.len());

        // Get Leaf hash, and leaf sibling hash,
        let leaf_index_in_tree = convert_index_to_last_level(index, tree_height);
        let leaf_sibling_hash = if index & 1 == 0 {
            // leaf is left child
            self.leaf_nodes[index + 1].clone()
        } else {
            // leaf is right child
            self.leaf_nodes[index - 1].clone()
        };

        // path.len() = `tree height - 2`, the two missing elements being the leaf sibling hash and the root
        let mut path = Vec::with_capacity(tree_height - 2);
        // Iterate from the bottom layer after the leaves, to the top, storing all sibling node's hash values.
        let mut current_node = parent(leaf_index_in_tree).unwrap();
        while !is_root(current_node) {
            let sibling_node = sibling(current_node).unwrap();
            path.push(self.non_leaf_nodes[sibling_node].clone());
            current_node = parent(current_node).unwrap();
        }

        debug_assert_eq!(path.len(), tree_height - 2);

        // we want to make path from root to bottom
        path.reverse();

        Ok(Path {
            leaf_index: index,
            auth_path: path,
            leaf_sibling_hash,
        })
    }

    /// Given the index and new leaf, return the hash of leaf and an updated path in order from root to bottom non-leaf level.
    /// This does not mutate the underlying tree.
    fn updated_path(
        &self,
        index: usize,
        new_leaf_hash: &Vec<u8>,
    ) -> Result<(Vec<u8>, Vec<Vec<u8>>), crate::Error> {
        // leaf is expected to be already hashed

        // calculate leaf sibling hash and locate its position (left or right)
        let (leaf_left, leaf_right) = if index & 1 == 0 {
            // leaf on left
            (new_leaf_hash.clone(), self.leaf_nodes[index + 1].clone())
        } else {
            (self.leaf_nodes[index - 1].clone(), new_leaf_hash.clone())
        };

        // calculate the updated hash at bottom non-leaf-level
        let mut path_bottom_to_top = Vec::with_capacity(self.height - 1);
        {
            path_bottom_to_top.push(hash_64_to_vec([&leaf_left[..], &leaf_right[..]].concat()).to_vec());
        }

        // then calculate the updated hash from bottom to root
        let leaf_index_in_tree = convert_index_to_last_level(index, self.height);
        let mut prev_index = parent(leaf_index_in_tree).unwrap();
        while !is_root(prev_index) {
            let (left_hash_bytes, right_hash_bytes) = if is_left_child(prev_index) {
                (
                    ark_ff::to_bytes!(path_bottom_to_top.last().unwrap())?,
                    ark_ff::to_bytes!(&self.non_leaf_nodes[sibling(prev_index).unwrap()])?,
                )
            } else {
                (
                    ark_ff::to_bytes!(&self.non_leaf_nodes[sibling(prev_index).unwrap()])?,
                    ark_ff::to_bytes!(path_bottom_to_top.last().unwrap())?,
                )
            };
            path_bottom_to_top.push(hash_64_to_vec([&left_hash_bytes[..], &right_hash_bytes[..]].concat()).to_vec());
            /*
                P::TwoToOneHash::evaluate(
                &self.two_to_one_hash_param,
                &left_hash_bytes,
                &right_hash_bytes,
            )?);*/
            prev_index = parent(prev_index).unwrap();
        }

        debug_assert_eq!(path_bottom_to_top.len(), self.height - 1);
        let path_top_to_bottom: Vec<_> = path_bottom_to_top.into_iter().rev().collect();
        Ok((new_leaf_hash.to_vec(), path_top_to_bottom))
    }

    /// Update the leaf at `index` to updated leaf.
    /// ```tree_diagram
    ///         [A]
    ///        /   \
    ///      [B]    C
    ///     / \   /  \
    ///    D [E] F    H
    ///   .. / \ ....
    ///    [I] J
    /// ```
    /// update(3, {new leaf}) would swap the leaf value at `[I]` and cause a recomputation of `[A]`, `[B]`, and `[E]`.
    pub fn update(&mut self, index: usize, new_leaf: &Vec<u8>) -> Result<(), crate::Error> {
        assert!(index < self.leaf_nodes.len(), "index out of range");
        let (updated_leaf_hash, mut updated_path) = self.updated_path(index, new_leaf)?;
        self.leaf_nodes[index] = updated_leaf_hash;
        let mut curr_index = convert_index_to_last_level(index, self.height);
        for _ in 0..self.height - 1 {
            curr_index = parent(curr_index).unwrap();
            self.non_leaf_nodes[curr_index] = updated_path.pop().unwrap();
        }
        Ok(())
    }
    /*
    /// Update the leaf and check if the updated root is equal to `asserted_new_root`.
    ///
    /// Tree will not be modified if the check fails.
    pub fn check_update<L: ToBytes>(
        &mut self,
        index: usize,
        new_leaf: &L,
        //asserted_new_root: &TwoToOneDigest<P>,
    ) -> Result<bool, crate::Error> {
        assert!(index < self.leaf_nodes.len(), "index out of range");
        let (updated_leaf_hash, mut updated_path) = self.updated_path(index, new_leaf)?;
        if &updated_path[0] != asserted_new_root {
            return Ok(false);
        }
        self.leaf_nodes[index] = updated_leaf_hash;
        let mut curr_index = convert_index_to_last_level(index, self.height);
        for _ in 0..self.height - 1 {
            curr_index = parent(curr_index).unwrap();
            self.non_leaf_nodes[curr_index] = updated_path.pop().unwrap();
        }
        Ok(true)
    }*/
}

/// Returns the height of the tree, given the number of leaves.
#[inline]
fn tree_height(num_leaves: usize) -> usize {
    if num_leaves == 1 {
        return 1;
    }

    (ark_std::log2(num_leaves) as usize) + 1
}
/// Returns true iff the index represents the root.
#[inline]
fn is_root(index: usize) -> bool {
    index == 0
}

/// Returns the index of the left child, given an index.
#[inline]
fn left_child(index: usize) -> usize {
    2 * index + 1
}

/// Returns the index of the right child, given an index.
#[inline]
fn right_child(index: usize) -> usize {
    2 * index + 2
}

/// Returns the index of the sibling, given an index.
#[inline]
fn sibling(index: usize) -> Option<usize> {
    if index == 0 {
        None
    } else if is_left_child(index) {
        Some(index + 1)
    } else {
        Some(index - 1)
    }
}

/// Returns true iff the given index represents a left child.
#[inline]
fn is_left_child(index: usize) -> bool {
    index % 2 == 1
}

/// Returns the index of the parent, given an index.
#[inline]
fn parent(index: usize) -> Option<usize> {
    if index > 0 {
        Some((index - 1) >> 1)
    } else {
        None
    }
}

#[inline]
fn convert_index_to_last_level(index: usize, tree_height: usize) -> usize {
    index + (1 << (tree_height - 1)) - 1
}




#[cfg(test)]
mod tests {
    use std::convert::TryInto;
    use ark_std::{UniformRand, test_rng};
    use crate::{merkle_tree, insert};
    use crate::arkworks_merkle_tree_poseidon_sponge::*;

    use ark_ff::bytes::{ToBytes, FromBytes};
    use ark_sponge;
    use ark_sponge::poseidon::PoseidonSponge;
    use ark_ff::Fp256;
    use ark_ff::BigInteger256;
    use ark_ed_on_bn254;
    use ark_sponge::CryptographicSponge;
    use crate::poseidon_params::get_params;
    use std::time::{Duration, SystemTime};
    use crate::HashBytes;

    #[test]
    fn merkle_tree_new_test() {

        let tree_height = 4;

        let zero_value = hash_64_to_vec(vec![1u8;64]).to_vec();

        let leaves: Vec<Vec<u8>> = vec![zero_value;16];
        let mut tree = MerkleTree::new(
            &leaves,
        ).unwrap();
        let leaf1: Vec<u8>  = hash_64_to_vec(vec![2u8;64]).to_vec();
        tree.update(0, &leaf1);
        let leaf2: Vec<u8>  = hash_64_to_vec(vec![3u8;64]).to_vec();
        tree.update(1, &leaf2);
        let leaf3: Vec<u8>  = hash_64_to_vec(vec![4u8;64]).to_vec();
        tree.update(2, &leaf3);
        let proof = tree.generate_proof(2).unwrap();
        println!("Proof: {:?}", proof);
        assert!(proof.verify(&tree.root(), &leaf3).unwrap());
    }

    #[test]
    fn merkle_tree_arkworks_fork_vs_tornado_cash_fork_init() {
        //testing full arkforks_merkle tree vs sparse tornado cash fork tree for heights from 1 to 15
        for i in 1..15 {
            println!("tree_height: {}", i);
            let now = SystemTime::now();
            let tree_height = i;
            let zero_value = [1u8, 32];
            let zero_value = vec![1 as u8;32];
            let mut smt = merkle_tree {is_initialized: true,
                levels: 1,
                filledSubtrees:vec![vec![0u8;32];1],
                zeros: vec![vec![0u8;32];1],
                currentRootIndex: 0,
                nextIndex: 0,
                ROOT_HISTORY_SIZE: 10,
                roots: vec![vec![0u8;32];1],
                leaves: vec![vec![0u8;32];1],
            };

            smt.initialize(tree_height, zero_value.clone());
            let leaves: Vec<Vec<u8>> = vec![smt.zeros[0].clone(); 2_usize.pow(tree_height.try_into().unwrap())];
            println!("starting to init arkworks_fork tree");
            let mut tree = MerkleTree::new(
                &leaves,
            ).unwrap();
            //println!("root: {:?}", tree.root());
            //assert_eq!(smt.levels, tree.height);
            assert_eq!(smt.roots[0], tree.root());
            let new_sys_time = SystemTime::now();
            println!("{:?}", new_sys_time.duration_since(now).unwrap());
        }

    }

    #[test]
    fn merkle_tree_arkworks_fork_vs_tornado_cash_fork_insert() {
        //testing full arkforks_merkle tree vs sparse tornado cash fork tree for heights from 1 to 15
        let tree_height = 11;
        println!("tree_height: {}", tree_height);
        let zero_value = [1u8, 32];
        let zero_value = vec![1 as u8;32];
        let mut smt = merkle_tree {is_initialized: true,
            levels: 1,
            filledSubtrees:vec![vec![0u8;32];1],
            zeros: vec![vec![0u8;32];1],
            currentRootIndex: 0,
            nextIndex: 0,
            ROOT_HISTORY_SIZE: 100,
            roots: vec![vec![0u8;32];100],
            leaves: vec![vec![0u8;32];2048],
        };

        smt.initialize(tree_height, zero_value.clone());
        let leaves: Vec<Vec<u8>> = vec![smt.zeros[0].clone(); 2_usize.pow(tree_height.try_into().unwrap())];
        println!("starting to init arkworks_fork tree");
        let mut tree = MerkleTree::new(
            &leaves,
        ).unwrap();
        //println!("root: {:?}", tree.root());
        //assert_eq!(smt.levels, tree.height);
        assert_eq!(smt.roots[0], tree.root());

        for i in 0..2 {
            let mut rng = test_rng();

            let mut hash_tmp_account = HashBytes {
                is_initialized: true,
                state_range_1: vec![0 as u8; 32],
                state_range_2: vec![0 as u8; 32],
                state_range_3: vec![0 as u8; 32],
                //result: vec![0 as u8; 32],
                left: vec![0 as u8; 32],
                right: vec![0 as u8; 32],
                currentLevelHash: vec![0 as u8; 32],
                currentIndex: 0,
                currentLevel:0,
            };



            let new_leaf_hash = Fp256::<ark_ed_on_bn254::FqParameters>::rand(&mut rng);

            let mut new_leaf_hash_bytes = vec![0u8;32];
            <Fp256::<ark_ed_on_bn254::FqParameters> as ToBytes>::write(&new_leaf_hash, &mut new_leaf_hash_bytes[..]);
            //println!("{:?}", new_leaf_bytes);
            //insert(&mut smt, &new_leaf_hash_bytes, &mut hash_tmp_account);
            insert(&new_leaf_hash_bytes.clone(),&mut smt, &mut hash_tmp_account);
            let filled_leaves = smt.leaves[..smt.nextIndex].to_vec().clone();
            let fll = &filled_leaves.len();

            let leaves: Vec<Vec<u8>> = [filled_leaves.clone(), vec![smt.zeros[0].clone(); 2_usize.pow(11u32)-fll]].concat();

            let mut tree_loop = MerkleTree::new(
                &leaves,
            ).unwrap();

            for (i, elem) in filled_leaves.iter().enumerate() {
                tree.update(i, elem);
            }

            //tree.update(i, &new_leaf_hash_bytes);
            assert_eq!(smt.roots[smt.currentRootIndex], tree.root(), "{}", i);
        }

    }

}
