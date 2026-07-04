const express = require("express");

const app = express();

const PORT = process.env.FRONTEND_PORT || 3000;

app.get("/", (req, res) => {
    res.send(`
        <h1>Hello Berth 👋</h1>

        <p>Frontend Port: ${PORT}</p>

        <p>Berth: ${process.env.BERTH_NAME}</p>
    `);
});

app.listen(PORT, () => {
    console.log(`Frontend listening on ${PORT}`);
});