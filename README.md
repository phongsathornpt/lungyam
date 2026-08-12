# Lungyam (ลุงยาม) 🛡️

![Rust](https://img.shields.io/badge/Rust-Black?style=flat-square&logo=rust&logoColor=white)
![Edge Native](https://img.shields.io/badge/Deployment-Edge%20Network-blue?style=flat-square)
![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)

**Lungyam** is a blazing-fast, edge-native API proxy built with **Rust**. 

Lungyam (ลุงยาม) คือซอฟต์แวร์ API Proxy ประสิทธิภาพสูงที่ถูกออกแบบมาเพื่อรันบนสภาพแวดล้อม **Edge Network** โดยเฉพาะ ด้วยพลังของภาษา Rust ทำให้ตัวโปรแกรมมีขนาดเล็ก ใช้ทรัพยากรน้อย และตอบสนองต่อ Request ได้ด้วยความหน่วง (Latency) ที่ต่ำที่สุด เหมาะสำหรับการเป็นหน้าด่านให้กับเซิร์ฟเวอร์หลักของคุณ

## ✨ Features

* 🚀 **Edge-Optimized:** ออกแบบมาเพื่อทำงานบน Edge nodes, Serverless หรือ Containerized environments ได้อย่างลื่นไหล
* ⚡ **Blazing Fast Performance:** จัดการทราฟฟิกมหาศาลได้โดยไม่มีปัญหาคอขวด ด้วยการจัดการหน่วยความจำที่ยอดเยี่ยมของ Rust (No Garbage Collection)
* 🪶 **Minimal Footprint:** ตัวไบนารีมีขนาดเล็ก ประหยัด RAM ทำให้สามารถ Deploy ได้ในพื้นที่ที่มีทรัพยากรจำกัด
* 🔒 **Secure API Gateway:** ทำหน้าที่เป็นหน้าด่าน (Guard) ในการคัดกรอง Request, จัดการ Rate limiting, และเพิ่มความปลอดภัยก่อนส่งทราฟฟิกไปยัง Backend
* 🔄 **Flexible Routing:** รองรับการทำ Request/Response transformation และ Dynamic routing ที่ระดับเครือข่ายขอบ (Edge)

## 🎯 Use Cases

* **Edge API Gateway:** กระจายโหลดและจัดการทราฟฟิกใกล้ตัวผู้ใช้ให้มากที่สุด
* **Security & Filtering:** ใช้ลุงยามเพื่อคัดกรอง Bad requests หรือทำ Payload validation
* **Offloading Backend:** ลดภาระการทำงานของเซิร์ฟเวอร์หลักด้วยการจัดการ Edge Caching หรือ Authentication ที่ Edge

## 🛠️ Getting Started

### Prerequisites
* [Rust toolchain](https://rustup.rs/) (edition 2021 or later)

### Installation

Clone the repository and build the project:

```bash
git clone [https://github.com/yourusername/lungyam.git](https://github.com/yourusername/lungyam.git)
cd lungyam
cargo build --release
