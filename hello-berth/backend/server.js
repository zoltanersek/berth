const express = require("express");

const app = express();

const PORT = process.env.BACKEND_PORT || 8000;

app.get("/", (req, res) => {
    res.json({
        message: "Hello from backend!",
        berth: process.env.BERTH_NAME
    });
});

app.listen(PORT, () => {
    console.log(`Backend listening on ${PORT}`);
});