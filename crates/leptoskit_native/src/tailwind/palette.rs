//! Default Tailwind 4.3 palette, converted from OKLCH to 8-bit sRGB.
//! Source: <https://github.com/tailwindlabs/tailwindcss/blob/v4.3.0/packages/tailwindcss/theme.css>
//! Theme data: MIT license, Copyright (c) Tailwind Labs, Inc.
//! Values outside sRGB are clipped consistently for every platform.
//!
//! MIT License
//!
//! Copyright (c) Tailwind Labs, Inc.
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to
//! deal in the Software without restriction, including without limitation the
//! rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
//! sell copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//! IN THE SOFTWARE.

use crate::Color;

/// The shade suffixes every palette family defines, in the order of each
/// family's colors in [`FAMILIES`].
const SHADES: [&str; 11] = [
	"50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950",
];

/// Each palette family's colors as packed RGBA, one per entry of [`SHADES`]:
/// the first row holds shades 50 to 500 and the second 600 to 950.
#[rustfmt::skip]
const FAMILIES: [(&str, [u32; 11]); 26] = [
	("red", [
		0xfef2_f2ff, 0xffe2_e2ff, 0xffc9_c9ff, 0xffa2_a2ff, 0xff64_67ff, 0xfb2c_36ff,
		0xe700_0bff, 0xc100_07ff, 0x9f07_12ff, 0x8218_1aff, 0x4608_09ff,
	]),
	("orange", [
		0xfff7_edff, 0xffed_d4ff, 0xffd6_a7ff, 0xffb8_6aff, 0xff89_04ff, 0xff69_00ff,
		0xf549_00ff, 0xca35_00ff, 0x9f2d_00ff, 0x7e2a_0cff, 0x4413_06ff,
	]),
	("amber", [
		0xfffb_ebff, 0xfef3_c6ff, 0xfee6_85ff, 0xffd2_30ff, 0xffb9_00ff, 0xfe9a_00ff,
		0xe171_00ff, 0xbb4d_00ff, 0x973c_00ff, 0x7b33_06ff, 0x4619_01ff,
	]),
	("yellow", [
		0xfefc_e8ff, 0xfef9_c2ff, 0xfff0_85ff, 0xffdf_20ff, 0xfdc7_00ff, 0xf0b1_00ff,
		0xd087_00ff, 0xa65f_00ff, 0x894b_00ff, 0x733e_0aff, 0x4320_04ff,
	]),
	("lime", [
		0xf7fe_e7ff, 0xecfc_caff, 0xd8f9_99ff, 0xbbf4_51ff, 0x9ae6_00ff, 0x7ccf_00ff,
		0x5ea5_00ff, 0x497d_00ff, 0x3c63_00ff, 0x3553_0eff, 0x192e_03ff,
	]),
	("green", [
		0xf0fd_f4ff, 0xdcfc_e7ff, 0xb9f8_cfff, 0x7bf1_a8ff, 0x05df_72ff, 0x00c9_50ff,
		0x00a6_3eff, 0x0082_36ff, 0x0166_30ff, 0x0d54_2bff, 0x032e_15ff,
	]),
	("emerald", [
		0xecfd_f5ff, 0xd0fa_e5ff, 0xa4f4_cfff, 0x5ee9_b5ff, 0x00d4_92ff, 0x00bc_7dff,
		0x0099_66ff, 0x007a_55ff, 0x0060_45ff, 0x004f_3bff, 0x002c_22ff,
	]),
	("teal", [
		0xf0fd_faff, 0xcbfb_f1ff, 0x96f7_e4ff, 0x46ec_d5ff, 0x00d5_beff, 0x00bb_a7ff,
		0x0096_89ff, 0x0078_6fff, 0x005f_5aff, 0x0b4f_4aff, 0x022f_2eff,
	]),
	("cyan", [
		0xecfe_ffff, 0xcefa_feff, 0xa2f4_fdff, 0x53ea_fdff, 0x00d3_f2ff, 0x00b8_dbff,
		0x0092_b8ff, 0x0075_95ff, 0x005f_78ff, 0x104e_64ff, 0x0533_45ff,
	]),
	("sky", [
		0xf0f9_ffff, 0xdff2_feff, 0xb8e6_feff, 0x74d4_ffff, 0x00bc_ffff, 0x00a6_f4ff,
		0x0084_d1ff, 0x0069_a8ff, 0x0059_8aff, 0x024a_70ff, 0x052f_4aff,
	]),
	("blue", [
		0xeff6_ffff, 0xdbea_feff, 0xbedb_ffff, 0x8ec5_ffff, 0x51a2_ffff, 0x2b7f_ffff,
		0x155d_fcff, 0x1447_e6ff, 0x193c_b8ff, 0x1c39_8eff, 0x1624_56ff,
	]),
	("indigo", [
		0xeef2_ffff, 0xe0e7_ffff, 0xc6d2_ffff, 0xa3b3_ffff, 0x7c86_ffff, 0x615f_ffff,
		0x4f39_f6ff, 0x432d_d7ff, 0x372a_acff, 0x312c_85ff, 0x1e1a_4dff,
	]),
	("violet", [
		0xf5f3_ffff, 0xede9_feff, 0xddd6_ffff, 0xc4b4_ffff, 0xa684_ffff, 0x8e51_ffff,
		0x7f22_feff, 0x7008_e7ff, 0x5d0e_c0ff, 0x4d17_9aff, 0x2f0d_68ff,
	]),
	("purple", [
		0xfaf5_ffff, 0xf3e8_ffff, 0xe9d4_ffff, 0xdab2_ffff, 0xc27a_ffff, 0xad46_ffff,
		0x9810_faff, 0x8200_dbff, 0x6e11_b0ff, 0x5916_8bff, 0x3c03_66ff,
	]),
	("fuchsia", [
		0xfdf4_ffff, 0xfae8_ffff, 0xf6cf_ffff, 0xf4a8_ffff, 0xed6a_ffff, 0xe12a_fbff,
		0xc800_deff, 0xa800_b7ff, 0x8a01_94ff, 0x7213_78ff, 0x4b00_4fff,
	]),
	("pink", [
		0xfdf2_f8ff, 0xfce7_f3ff, 0xfcce_e8ff, 0xfda5_d5ff, 0xfb64_b6ff, 0xf633_9aff,
		0xe600_76ff, 0xc600_5cff, 0xa300_4cff, 0x8610_43ff, 0x5104_24ff,
	]),
	("rose", [
		0xfff1_f2ff, 0xffe4_e6ff, 0xffcc_d3ff, 0xffa1_adff, 0xff63_7eff, 0xff20_56ff,
		0xec00_3fff, 0xc700_36ff, 0xa500_36ff, 0x8b08_36ff, 0x4d02_18ff,
	]),
	("slate", [
		0xf8fa_fcff, 0xf1f5_f9ff, 0xe2e8_f0ff, 0xcad5_e2ff, 0x90a1_b9ff, 0x6274_8eff,
		0x4555_6cff, 0x3141_58ff, 0x1d29_3dff, 0x0f17_2bff, 0x0206_18ff,
	]),
	("gray", [
		0xf9fa_fbff, 0xf3f4_f6ff, 0xe5e7_ebff, 0xd1d5_dcff, 0x99a1_afff, 0x6a72_82ff,
		0x4a55_65ff, 0x3641_53ff, 0x1e29_39ff, 0x1018_28ff, 0x0307_12ff,
	]),
	("zinc", [
		0xfafa_faff, 0xf4f4_f5ff, 0xe4e4_e7ff, 0xd4d4_d8ff, 0x9f9f_a9ff, 0x7171_7bff,
		0x5252_5cff, 0x3f3f_46ff, 0x2727_2aff, 0x1818_1bff, 0x0909_0bff,
	]),
	("neutral", [
		0xfafa_faff, 0xf5f5_f5ff, 0xe5e5_e5ff, 0xd4d4_d4ff, 0xa1a1_a1ff, 0x7373_73ff,
		0x5252_52ff, 0x4040_40ff, 0x2626_26ff, 0x1717_17ff, 0x0a0a_0aff,
	]),
	("stone", [
		0xfafa_f9ff, 0xf5f5_f4ff, 0xe7e5_e4ff, 0xd6d3_d1ff, 0xa6a0_9bff, 0x7971_6bff,
		0x5753_4dff, 0x4440_3bff, 0x2925_24ff, 0x1c19_17ff, 0x0c0a_09ff,
	]),
	("mauve", [
		0xfafa_faff, 0xf3f1_f3ff, 0xe7e4_e7ff, 0xd7d0_d7ff, 0xa89e_a9ff, 0x7969_7bff,
		0x594c_5bff, 0x4639_47ff, 0x2a21_2cff, 0x1d16_1eff, 0x0c09_0cff,
	]),
	("olive", [
		0xfbfb_f9ff, 0xf4f4_f0ff, 0xe8e8_e3ff, 0xd8d8_d0ff, 0xabab_9cff, 0x7c7c_67ff,
		0x5b5b_4bff, 0x4747_39ff, 0x2b2b_22ff, 0x1d1d_16ff, 0x0c0c_09ff,
	]),
	("mist", [
		0xf9fb_fbff, 0xf1f3_f3ff, 0xe3e7_e8ff, 0xd0d6_d8ff, 0x9ca8_abff, 0x6778_7cff,
		0x4b58_5bff, 0x3944_47ff, 0x2229_2bff, 0x161b_1dff, 0x090b_0cff,
	]),
	("taupe", [
		0xfbfa_f9ff, 0xf3f1_f1ff, 0xe8e4_e3ff, 0xd8d2_d0ff, 0xaba0_9cff, 0x7c6d_67ff,
		0x5b4f_4bff, 0x473c_39ff, 0x2b24_22ff, 0x1d18_16ff, 0x0c0a_09ff,
	]),
];

/// The default palette color a `family-shade` name, such as `red-500`, names.
pub(super) fn lookup(name: &str) -> Option<Color> {
	let (family, shade) = name.rsplit_once('-')?;
	let shade = SHADES.iter().position(|&candidate| candidate == shade)?;
	let (_, colors) = FAMILIES
		.iter()
		.find(|(candidate, _)| *candidate == family)?;

	colors.get(shade).copied().map(Color)
}
